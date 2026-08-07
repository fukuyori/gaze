use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use eframe::egui::{Context, ViewportCommand};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

use crate::{
    animation::BlinkRate,
    settings::{OPACITY_CHOICES, Settings},
};

const ACTION_TOGGLE: u8 = 1;
const ACTION_AUTOSTART: u8 = 2;
const ACTION_ALWAYS_ON_TOP: u8 = 4;
const ACTION_OPACITY: u8 = 8;
const ACTION_BLINK: u8 = 16;

pub struct TrayState {
    _icon: TrayIcon,
    toggle_item: MenuItem,
    autostart_item: CheckMenuItem,
    always_on_top_item: CheckMenuItem,
    opacity_items: Vec<(CheckMenuItem, u8)>,
    blink_items: Vec<(CheckMenuItem, BlinkRate)>,
    pending_action: Arc<AtomicU8>,
    requested_opacity: Arc<AtomicU8>,
    /// Index into `BlinkRate::ALL`, the menu handler's way of naming a rate.
    requested_blink: Arc<AtomicU8>,
    window_visible: bool,
}

impl TrayState {
    pub fn new(
        ctx: &Context,
        exit_placement: crate::placement::SharedPlacement,
        settings: Settings,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let menu = Menu::new();
        let toggle_item = MenuItem::new("Hide", true, None);
        let always_on_top_item =
            CheckMenuItem::new("Always on top", true, settings.always_on_top, None);
        let opacity_items: Vec<(CheckMenuItem, u8)> = OPACITY_CHOICES
            .iter()
            .map(|&percent| {
                let item = CheckMenuItem::new(
                    format!("Opacity {percent}%"),
                    true,
                    percent == settings.opacity_percent,
                    None,
                );
                (item, percent)
            })
            .collect();
        let blink_items: Vec<(CheckMenuItem, BlinkRate)> = BlinkRate::ALL
            .into_iter()
            .map(|rate| {
                let item = CheckMenuItem::new(
                    format!("Blinking: {}", rate.label()),
                    true,
                    rate == settings.blink,
                    None,
                );
                (item, rate)
            })
            .collect();
        let autostart_item = CheckMenuItem::new(
            "Start with Windows",
            true,
            crate::autostart::is_enabled().unwrap_or(false),
            None,
        );
        let quit_item = MenuItem::new("Exit", true, None);
        menu.append_items(&[&toggle_item, &always_on_top_item])?;
        menu.append(&PredefinedMenuItem::separator())?;
        for (item, _) in &blink_items {
            menu.append(item)?;
        }
        menu.append(&PredefinedMenuItem::separator())?;
        for (item, _) in &opacity_items {
            menu.append(item)?;
        }
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append_items(&[
            &autostart_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])?;

        let pending_action = Arc::new(AtomicU8::new(0));
        let requested_opacity = Arc::new(AtomicU8::new(settings.opacity_percent));
        let requested_blink = Arc::new(AtomicU8::new(blink_index(settings.blink)));
        let handler_action = Arc::clone(&pending_action);
        let handler_opacity = Arc::clone(&requested_opacity);
        let handler_blink = Arc::clone(&requested_blink);
        let handler_ctx = ctx.clone();
        let toggle_id = toggle_item.id().clone();
        let autostart_id = autostart_item.id().clone();
        let always_on_top_id = always_on_top_item.id().clone();
        let opacity_ids: Vec<_> = opacity_items
            .iter()
            .map(|(item, percent)| (item.id().clone(), *percent))
            .collect();
        let blink_ids: Vec<_> = blink_items
            .iter()
            .enumerate()
            .map(|(index, (item, _))| (item.id().clone(), index as u8))
            .collect();
        let quit_id = quit_item.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == toggle_id {
                handler_action.fetch_or(ACTION_TOGGLE, Ordering::Release);
                handler_ctx.request_repaint();
            } else if event.id == autostart_id {
                handler_action.fetch_or(ACTION_AUTOSTART, Ordering::Release);
                handler_ctx.request_repaint();
            } else if event.id == always_on_top_id {
                handler_action.fetch_or(ACTION_ALWAYS_ON_TOP, Ordering::Release);
                handler_ctx.request_repaint();
            } else if let Some((_, percent)) = opacity_ids.iter().find(|(id, _)| *id == event.id) {
                handler_opacity.store(*percent, Ordering::Release);
                handler_action.fetch_or(ACTION_OPACITY, Ordering::Release);
                handler_ctx.request_repaint();
            } else if let Some((_, index)) = blink_ids.iter().find(|(id, _)| *id == event.id) {
                handler_blink.store(*index, Ordering::Release);
                handler_action.fetch_or(ACTION_BLINK, Ordering::Release);
                handler_ctx.request_repaint();
            } else if event.id == quit_id {
                let _ = exit_placement.save_latest();
                std::process::exit(0);
            }
        }));

        let icon = TrayIconBuilder::new()
            .with_tooltip("Gaze")
            .with_icon(create_eye_icon()?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_menu_on_right_click(true)
            .build()?;

        Ok(Self {
            _icon: icon,
            toggle_item,
            autostart_item,
            always_on_top_item,
            opacity_items,
            blink_items,
            pending_action,
            requested_opacity,
            requested_blink,
            window_visible: true,
        })
    }

    /// Applies the pending menu clicks, returning whether `settings` changed.
    pub fn handle_pending_action(&mut self, ctx: &Context, settings: &mut Settings) -> bool {
        let action = self.pending_action.swap(0, Ordering::AcqRel);
        if action & ACTION_TOGGLE != 0 {
            self.window_visible = !self.window_visible;
            ctx.send_viewport_cmd(ViewportCommand::Visible(self.window_visible));
            self.toggle_item
                .set_text(if self.window_visible { "Hide" } else { "Show" });
        }
        if action & ACTION_AUTOSTART != 0 {
            let enabled = self.autostart_item.is_checked();
            if crate::autostart::set_enabled(enabled).is_err() {
                self.autostart_item.set_checked(!enabled);
            }
        }

        let mut changed = false;
        if action & ACTION_ALWAYS_ON_TOP != 0 {
            settings.always_on_top = self.always_on_top_item.is_checked();
            changed = true;
        }
        if action & ACTION_OPACITY != 0 {
            settings.opacity_percent = self.requested_opacity.load(Ordering::Acquire);
            // The menu items are a radio group, so only the chosen one stays ticked.
            for (item, percent) in &self.opacity_items {
                item.set_checked(*percent == settings.opacity_percent);
            }
            changed = true;
        }
        if action & ACTION_BLINK != 0 {
            let index = self.requested_blink.load(Ordering::Acquire) as usize;
            settings.blink = BlinkRate::ALL[index.min(BlinkRate::ALL.len() - 1)];
            for (item, rate) in &self.blink_items {
                item.set_checked(*rate == settings.blink);
            }
            changed = true;
        }
        changed
    }
}

fn blink_index(rate: BlinkRate) -> u8 {
    BlinkRate::ALL
        .iter()
        .position(|candidate| *candidate == rate)
        .unwrap_or(0) as u8
}

fn create_eye_icon() -> Result<Icon, tray_icon::BadIcon> {
    const SIZE: u32 = 32;
    const SAMPLES: u32 = 4;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut total = [0_u32; 4];
            for sample_y in 0..SAMPLES {
                for sample_x in 0..SAMPLES {
                    let px = x as f32 + (sample_x as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (sample_y as f32 + 0.5) / SAMPLES as f32;
                    let color = icon_sample(px, py);
                    for channel in 0..4 {
                        total[channel] += color[channel] as u32;
                    }
                }
            }
            let divisor = SAMPLES * SAMPLES;
            rgba.extend(total.map(|channel| (channel / divisor) as u8));
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE)
}

fn icon_sample(x: f32, y: f32) -> [u8; 4] {
    for center_x in [9.5, 22.5] {
        let eye_distance = ((x - center_x) / 7.8).powi(2) + ((y - 16.0) / 11.5).powi(2);
        if eye_distance <= 1.0 {
            let pupil_distance = ((x - (center_x + 1.3)).powi(2) + (y - 17.0).powi(2)).sqrt();
            return if pupil_distance <= 1.8 {
                [24, 31, 36, 255]
            } else if pupil_distance <= 3.4 {
                [73, 137, 154, 255]
            } else if eye_distance >= 0.76 {
                [39, 38, 42, 255]
            } else {
                [255, 254, 250, 255]
            };
        }
    }
    [0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_has_expected_rgba_size() {
        assert!(create_eye_icon().is_ok());
    }

    #[test]
    fn tray_icon_corners_are_transparent() {
        assert_eq!(icon_sample(0.0, 0.0), [0, 0, 0, 0]);
    }
}
