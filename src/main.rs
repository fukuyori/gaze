#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod animation;
#[cfg(any(windows, target_os = "macos"))]
mod autostart;
#[cfg(windows)]
mod input_activity;
#[cfg(target_os = "macos")]
mod macos_cursor;
#[cfg(target_os = "macos")]
mod macos_window;
#[cfg(target_os = "macos")]
mod manual_resize;
#[cfg(windows)]
mod native_resize;
#[cfg(windows)]
mod placement;
mod repaint_watchdog;
mod settings;
#[cfg(windows)]
mod tray;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use animation::{Blinker, GazeTracker, IdleAnimator, distance_squint, pupil_offset};
use eframe::egui::{
    self, Color32, CursorIcon, PointerButton, Pos2, Rect, ResizeDirection, Shape, Stroke, Vec2,
    ViewportBuilder, ViewportCommand, viewport::WindowLevel,
};
use settings::Settings;

const OUTLINE: Color32 = Color32::from_rgb(39, 38, 42);
const SCLERA: Color32 = Color32::from_rgb(255, 254, 250);
const IRIS: Color32 = Color32::from_rgb(73, 137, 154);
const PUPIL: Color32 = Color32::from_rgb(24, 31, 36);
const MOUTH: Color32 = Color32::from_rgb(68, 39, 46);
const TONGUE: Color32 = Color32::from_rgb(205, 104, 119);

/// Iris size relative to the eye, and the smallest it is ever drawn. Both eye
/// half-axes feed into it so that narrowing the window shrinks the iris exactly
/// as much as flattening it does. The floor is only there to keep the iris
/// drawable: the ratio is what keeps it from looking like a speck.
const IRIS_RATIO: f32 = 0.38;
const IRIS_MIN_RADIUS: f32 = 5.0;

/// Half the gap between the eyes, which is also the room the outlines need
/// outside them.
const EYE_GAP: f32 = 3.0;
/// How much of the window's width the face spans. Everything is proportional so
/// that a small window shows a small face rather than one cropped by the edges.
const FACE_WIDTH_RATIO: f32 = 0.9;
const FACE_HEIGHT_RATIO: f32 = 0.35;

/// The face is `4 * radius_x + 2 * EYE_GAP` wide, so this is that read
/// backwards from the share of the window the face is allowed.
fn eye_radius_x(window_width: f32) -> f32 {
    ((window_width * FACE_WIDTH_RATIO - 2.0 * EYE_GAP) / 4.0).clamp(6.0, 125.0)
}

fn eye_radius_y(window_height: f32) -> f32 {
    (window_height * FACE_HEIGHT_RATIO).clamp(6.0, 110.0)
}

/// Room left around the settings menu for the popup's own frame, so egui can
/// still fit it inside the window.
#[cfg(not(windows))]
const MENU_MARGIN: f32 = 28.0;

/// How close to the face's edge the pointer starts resizing rather than
/// dragging. A fixed band would swallow a small face whole.
fn resize_edge(face_bounds: Rect) -> f32 {
    (face_bounds.height() * 0.12).clamp(4.0, 10.0)
}

#[derive(Clone, Copy)]
struct EyeShape {
    radius_x: f32,
    radius_y: f32,
    iris_radius: f32,
    openness: f32,
    narrowing: f32,
}

fn iris_radius(radius_x: f32, radius_y: f32) -> f32 {
    (IRIS_RATIO * (radius_x * radius_y).sqrt())
        // Keep the iris inside a short eye before honouring the lower bound, so
        // a clamp can never invert.
        .min(radius_y * 0.62)
        .max(IRIS_MIN_RADIUS)
}

fn main() -> eframe::Result {
    #[cfg(windows)]
    let restored_placement = placement::WindowPlacement::load().ok().flatten();
    #[cfg(windows)]
    let centered = restored_placement.is_none();
    #[cfg(not(windows))]
    let centered = true;

    let settings = Settings::load();
    let viewport = ViewportBuilder::default()
        .with_title("Gaze")
        .with_inner_size([placement_default_width(), placement_default_height()])
        .with_min_inner_size([placement_min_width(), placement_min_height()])
        .with_max_inner_size([placement_max_width(), placement_max_height()])
        .with_resizable(true)
        .with_transparent(true)
        .with_decorations(false)
        .with_taskbar(false)
        .with_has_shadow(false)
        .with_window_level(window_level(settings));

    #[cfg(windows)]
    let viewport = match restored_placement {
        Some(restored) => viewport
            .with_position([restored.x as f32, restored.y as f32])
            .with_inner_size(restored.restored_size()),
        None => viewport,
    };

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered,
        ..Default::default()
    };

    eframe::run_native(
        "Gaze",
        options,
        Box::new(move |creation_context| {
            GazeApp::new(creation_context, settings)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}

const fn window_level(settings: Settings) -> WindowLevel {
    if settings.always_on_top {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    }
}

struct GazeApp {
    started_at: Instant,
    blinker: Blinker,
    idle_animator: IdleAnimator,
    gaze: GazeTracker,
    last_cursor: Option<Pos2>,
    settings: Settings,
    _repaint_watchdog: repaint_watchdog::RepaintWatchdog,
    #[cfg(target_os = "macos")]
    resize_drag: Option<manual_resize::ResizeDrag>,
    #[cfg(target_os = "macos")]
    start_at_login: bool,
    #[cfg(target_os = "macos")]
    ns_view: usize,
    #[cfg(windows)]
    tray: tray::TrayState,
    #[cfg(windows)]
    window_handle: isize,
    #[cfg(windows)]
    placement_tracker: placement::PlacementTracker,
    #[cfg(windows)]
    native_resize: native_resize::NativeResize,
    #[cfg(windows)]
    keyboard_activity: input_activity::KeyboardActivity,
}

impl GazeApp {
    fn new(
        creation_context: &eframe::CreationContext<'_>,
        settings: Settings,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        creation_context
            .egui_ctx
            .set_visuals(egui::Visuals::light());
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        #[cfg(windows)]
        let window_handle = windows_handle(creation_context)?;
        #[cfg(windows)]
        let placement_tracker =
            placement::PlacementTracker::new(placement::WindowPlacement::load().ok().flatten());
        #[cfg(windows)]
        let tray = tray::TrayState::new(
            &creation_context.egui_ctx,
            placement_tracker.shared(),
            settings,
        )?;
        #[cfg(windows)]
        let keyboard_activity = input_activity::KeyboardActivity::new(window_handle)?;
        #[cfg(windows)]
        apply_windows_opacity(window_handle, settings.opacity_percent);
        #[cfg(target_os = "macos")]
        let ns_view = appkit_view(creation_context);
        #[cfg(target_os = "macos")]
        macos_window::set_on_all_desktops(ns_view, settings.show_on_all_desktops);
        Ok(Self {
            started_at: Instant::now(),
            blinker: Blinker::new(seed),
            idle_animator: IdleAnimator::new(seed.rotate_left(17)),
            gaze: GazeTracker::default(),
            last_cursor: None,
            settings,
            #[cfg(target_os = "macos")]
            resize_drag: None,
            #[cfg(target_os = "macos")]
            start_at_login: autostart::is_enabled().unwrap_or(false),
            #[cfg(target_os = "macos")]
            ns_view,
            #[cfg(windows)]
            _repaint_watchdog: repaint_watchdog::RepaintWatchdog::new(
                &creation_context.egui_ctx,
                window_handle,
            )?,
            #[cfg(not(windows))]
            _repaint_watchdog: repaint_watchdog::RepaintWatchdog::new(&creation_context.egui_ctx)?,
            #[cfg(windows)]
            tray,
            #[cfg(windows)]
            window_handle,
            #[cfg(windows)]
            placement_tracker,
            #[cfg(windows)]
            native_resize: native_resize::NativeResize::new(window_handle)?,
            #[cfg(windows)]
            keyboard_activity,
        })
    }

    fn cursor_in_viewport(&mut self, ctx: &egui::Context) -> Pos2 {
        #[cfg(windows)]
        if let Some(cursor) = windows_cursor_in_viewport(ctx) {
            self.last_cursor = Some(cursor);
            return cursor;
        }

        #[cfg(target_os = "macos")]
        if let Some(cursor) = desktop_cursor_in_viewport(ctx) {
            self.last_cursor = Some(cursor);
            return cursor;
        }

        if let Some(cursor) = ctx.input(|input| input.pointer.hover_pos()) {
            self.last_cursor = Some(cursor);
        }

        self.last_cursor.unwrap_or_else(|| {
            let rect = ctx.content_rect();
            rect.center()
        })
    }

    fn paint(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let rect = ui.max_rect();

        let cursor = self.cursor_in_viewport(ctx);
        let (monitor_diagonal, delta_time) = ctx.input(|input| {
            (
                input
                    .viewport()
                    .monitor_size
                    .unwrap_or(rect.size())
                    .length(),
                input.stable_dt,
            )
        });
        // The eyes aim at a trailing point rather than the cursor itself, so
        // the squint eases in with the same motion the irises follow.
        let gaze = self.gaze.follow(cursor, delta_time);
        let distance = (gaze - rect.center()).length();
        let squint = distance_squint(distance, monitor_diagonal);
        let elapsed = self.started_at.elapsed();
        #[cfg(windows)]
        let input_marker = Some(self.keyboard_activity.marker());
        #[cfg(not(windows))]
        let input_marker = keyboard_activity_marker(ctx, elapsed);
        let idle = self.idle_animator.expression(elapsed, cursor, input_marker);
        let blink = self.blinker.closure(elapsed, self.settings.blink);
        let openness =
            ((1.0 - 0.62 * squint) * (1.0 - blink) * (1.0 - 0.18 * idle.yawn) * (1.0 - idle.sleep))
                .clamp(0.0, 1.0);

        let scale = (rect.height() / 220.0).clamp(0.3, 1.46);
        let radius_x = eye_radius_x(rect.width());
        let base_radius_y = eye_radius_y(rect.height());
        let radius_y = base_radius_y * openness;
        // Leave room for the 4-point outlines so the two eyes do not overlap.
        // Pupil travel includes the same half-gap and can still reach center.
        let spacing = radius_x + EYE_GAP;
        let eye_center = rect.center() - Vec2::new(0.0, 18.0 * scale * idle.yawn);
        let left = eye_center - Vec2::new(spacing, 0.0);
        let right = eye_center + Vec2::new(spacing, 0.0);
        let mouth = (idle.yawn > 0.01).then(|| {
            (
                Pos2::new(rect.center().x, rect.bottom() - 30.0 * scale),
                Vec2::new(30.0 * scale, (4.0 + 20.0 * idle.yawn) * scale),
            )
        });
        let mut face_bounds = Rect::from_min_max(
            Pos2::new(
                left.x - radius_x - EYE_GAP,
                eye_center.y - radius_y - EYE_GAP,
            ),
            Pos2::new(
                right.x + radius_x + EYE_GAP,
                eye_center.y + radius_y + EYE_GAP,
            ),
        );
        if let Some((mouth_center, mouth_radius)) = mouth {
            face_bounds =
                face_bounds.union(Rect::from_center_size(mouth_center, 2.0 * mouth_radius));
        }
        // One band for both paths, so the cursor shape egui shows always agrees
        // with the area Windows treats as a resize border.
        let edge = resize_edge(face_bounds);
        #[cfg(windows)]
        self.native_resize
            .update(face_bounds, edge, ctx.pixels_per_point(), openness > 0.15);
        self.handle_window_interaction(ui, ctx, face_bounds, edge, openness);

        #[cfg(windows)]
        apply_windows_eye_region(
            self.window_handle,
            ctx.pixels_per_point(),
            FaceRegion {
                left_center: left,
                right_center: right,
                radius_x,
                radius_y,
                openness,
                mouth,
            },
        );

        let painter = ui.painter();
        // The iris keeps the size the fully open eye gives it: a blink or a
        // squint closes the lids over it instead of shrinking it.
        let eye = EyeShape {
            radius_x,
            radius_y,
            iris_radius: iris_radius(radius_x, base_radius_y),
            openness,
            narrowing: squint,
        };
        self.paint_eye(painter, left, gaze, eye);
        self.paint_eye(painter, right, gaze, eye);
        if let Some((center, radius)) = mouth {
            self.paint_mouth(painter, center, radius, idle.yawn);
        }
    }

    fn paint_eye(&self, painter: &egui::Painter, center: Pos2, gaze: Pos2, eye: EyeShape) {
        let EyeShape {
            radius_x,
            radius_y,
            iris_radius,
            openness,
            narrowing,
        } = eye;
        let alpha = self.face_alpha();
        if openness < 0.075 || radius_y < 3.0 {
            let half_width = radius_x * 0.82;
            painter.line_segment(
                [
                    center - Vec2::new(half_width, 0.0),
                    center + Vec2::new(half_width, 0.0),
                ],
                Stroke::new(5.0, shade(OUTLINE, alpha)),
            );
            return;
        }

        // The eye is drawn as an explicit polygon rather than with
        // `Shape::ellipse_filled`. egui tessellates an ellipse into as few as
        // eight segments per quarter, and that polygon sits *inside* the true
        // ellipse — an iris trimmed to the ellipse would show past the eye it
        // is supposed to be behind. Sharing one outline keeps the two exact.
        let eye_radius = Vec2::new(radius_x, radius_y.max(3.0));
        let eye_outline = ellipse_outline(center, eye_radius);
        painter.add(Shape::convex_polygon(
            eye_outline.clone(),
            shade(SCLERA, alpha),
            Stroke::NONE,
        ));

        // Distance squinting changes the eyelid opening, not the iris size:
        // the iris slides under the lid and is trimmed to the eye below.
        let offset = pupil_offset(gaze - center, radius_x, radius_y, iris_radius, narrowing);
        // The lids are the eye's own curve, so the iris is trimmed to it: a
        // rectangular clip between the lids lets an iris looking into a corner
        // slip out past the side of the eye. Trimming stops at the *inner* edge
        // of the outline rather than at its middle, because a translucent Gaze
        // shows whatever is painted underneath the lid straight through it.
        let iris_bounds = (eye_radius - Vec2::splat(EYE_OUTLINE_WIDTH / 2.0)).max(Vec2::splat(0.5));
        let iris_center = center + kept_inside(offset, iris_bounds);
        let trimmed = |disc_center: Pos2, radius: f32, color: Color32| {
            let outline = disc_inside_ellipse(center, iris_bounds, disc_center, radius);
            if outline.len() >= 3 {
                painter.add(Shape::convex_polygon(outline, color, Stroke::NONE));
            }
        };

        trimmed(iris_center, iris_radius, shade(IRIS, alpha));
        trimmed(iris_center, iris_radius * 0.54, shade(PUPIL, alpha));
        trimmed(
            iris_center - Vec2::splat(iris_radius * 0.27),
            (iris_radius * 0.16).max(1.5),
            shade(Color32::WHITE, alpha),
        );

        painter.add(Shape::convex_polygon(
            eye_outline,
            Color32::TRANSPARENT,
            Stroke::new(EYE_OUTLINE_WIDTH, shade(OUTLINE, alpha)),
        ));
    }

    fn paint_mouth(&self, painter: &egui::Painter, center: Pos2, radius: Vec2, openness: f32) {
        let alpha = self.face_alpha();
        let scale = radius.x / 30.0;
        painter.add(Shape::ellipse_filled(center, radius, shade(OUTLINE, alpha)));
        let inner_radius = Vec2::new(
            radius.x - 5.0 * scale,
            (radius.y - 5.0 * scale).max(1.5 * scale),
        );
        painter.add(Shape::ellipse_filled(
            center,
            inner_radius,
            shade(MOUTH, alpha),
        ));

        if openness > 0.38 {
            let tongue_radius = Vec2::new(12.0, 2.5 + 2.5 * openness) * scale;
            let tongue_center = center + Vec2::new(0.0, inner_radius.y * 0.48);
            painter.add(Shape::ellipse_filled(
                tongue_center,
                tongue_radius,
                shade(TONGUE, alpha),
            ));
        }
    }

    /// Windows fades the whole window through its layered-window alpha, so the
    /// painted colours only carry the opacity setting on other platforms.
    fn face_alpha(&self) -> f32 {
        if cfg!(windows) {
            1.0
        } else {
            self.settings.opacity()
        }
    }

    fn handle_window_interaction(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        face_bounds: Rect,
        edge: f32,
        openness: f32,
    ) {
        let response = ui.interact(
            ui.max_rect(),
            ui.id().with("window-drag-or-resize"),
            egui::Sense::click_and_drag(),
        );

        #[cfg(not(windows))]
        let menu_open = self.show_settings_menu(ctx, &response);
        #[cfg(windows)]
        let menu_open = false;

        let direction = if openness > 0.15 {
            ctx.pointer_hover_pos().and_then(|pointer| {
                let left = (pointer.x - face_bounds.left()).abs();
                let right = (pointer.x - face_bounds.right()).abs();
                let top = (pointer.y - face_bounds.top()).abs();
                let bottom = (pointer.y - face_bounds.bottom()).abs();
                let horizontal = left.min(right);
                let vertical = top.min(bottom);

                if horizontal < edge && horizontal <= vertical {
                    Some(if left <= right {
                        ResizeDirection::West
                    } else {
                        ResizeDirection::East
                    })
                } else if vertical < edge {
                    Some(if top <= bottom {
                        ResizeDirection::North
                    } else {
                        ResizeDirection::South
                    })
                } else {
                    None
                }
            })
        } else {
            None
        };

        if let Some(direction) = direction
            && !menu_open
        {
            ctx.set_cursor_icon(match direction {
                ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
                ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
                _ => CursorIcon::Default,
            });
        }

        // winit cannot start a native resize drag on macOS, so the window
        // resizes itself there while the pointer is held down.
        #[cfg(target_os = "macos")]
        if self.update_manual_resize(ctx, direction.filter(|_| !menu_open)) {
            return;
        }

        #[cfg(not(any(windows, target_os = "macos")))]
        if let Some(direction) = direction.filter(|_| !menu_open)
            && response.hovered()
            && ctx.input(|input| input.pointer.button_pressed(PointerButton::Primary))
        {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }

        if direction.is_none()
            && !menu_open
            && response.hovered()
            && ctx.input(|input| input.pointer.button_pressed(PointerButton::Primary))
        {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    /// Drives an in-progress edge resize, returning whether one is active.
    #[cfg(target_os = "macos")]
    fn update_manual_resize(
        &mut self,
        ctx: &egui::Context,
        direction: Option<ResizeDirection>,
    ) -> bool {
        let (pressed, held) = ctx.input(|input| {
            (
                input.pointer.button_pressed(PointerButton::Primary),
                input.pointer.button_down(PointerButton::Primary),
            )
        });
        if !held {
            self.resize_drag = None;
            return false;
        }

        // The desktop cursor, not the window-relative one: a window-relative
        // reading moves with the window this drag is moving.
        let Some(pointer) = macos_cursor::desktop_position() else {
            return self.resize_drag.is_some();
        };

        if self.resize_drag.is_none() {
            let Some(direction) = direction.filter(|_| pressed) else {
                return false;
            };
            let Some(window) = ctx.input(|input| input.viewport().outer_rect) else {
                return false;
            };
            self.resize_drag = Some(manual_resize::ResizeDrag::start(direction, window, pointer));
        }

        let Some(drag) = self.resize_drag else {
            return false;
        };
        let target = drag.resolve(
            pointer,
            Vec2::new(placement_min_width(), placement_min_height()),
            Vec2::new(placement_max_width(), placement_max_height()),
        );
        // Both commands go out every frame and land in the same run loop turn,
        // so the origin is restored within the same redraw if resizing the
        // content view shifted it.
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(target.size()));
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(target.min));
        true
    }

    /// Right-click menu with the settings that have no tray icon outside Windows.
    #[cfg(not(windows))]
    fn show_settings_menu(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        let mut settings = self.settings;
        #[cfg(target_os = "macos")]
        let mut start_at_login = self.start_at_login;

        response.context_menu(|ui| {
            ui.set_min_width(228.0);
            // egui keeps a popup inside the window, and Gaze can be dragged
            // down to a window shorter than any menu, so the list has to be
            // able to scroll rather than lose its bottom rows.
            egui::ScrollArea::vertical()
                .max_height((ui.ctx().content_rect().height() - MENU_MARGIN).max(48.0))
                .show(ui, |ui| {
                    ui.checkbox(&mut settings.always_on_top, "Always on top");
                    #[cfg(target_os = "macos")]
                    ui.checkbox(&mut settings.show_on_all_desktops, "Show on all desktops");
                    #[cfg(target_os = "macos")]
                    ui.checkbox(&mut start_at_login, "Start at login");
                    ui.separator();

                    // A row apiece rather than a column of radio buttons: eight
                    // stacked choices are taller than the window they open in.
                    ui.horizontal(|ui| {
                        ui.label("Blinking");
                        for rate in animation::BlinkRate::ALL {
                            ui.selectable_value(&mut settings.blink, rate, rate.label());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Opacity");
                        for percent in settings::OPACITY_CHOICES {
                            ui.selectable_value(
                                &mut settings.opacity_percent,
                                percent,
                                format!("{percent}\u{2009}%"),
                            );
                        }
                    });

                    ui.separator();
                    // Gaze keeps itself out of the Dock and the app switcher,
                    // so this is the only way out that does not need it focused.
                    if ui.button("Quit Gaze").clicked() {
                        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                    }
                });
        });

        if settings != self.settings {
            self.settings = settings;
            self.apply_settings(ctx);
            let _ = self.settings.save();
        }
        // The tick only moves once the login item is really there, so a failed
        // write shows as the setting staying off rather than a silent lie.
        #[cfg(target_os = "macos")]
        if start_at_login != self.start_at_login && autostart::set_enabled(start_at_login).is_ok() {
            self.start_at_login = start_at_login;
        }
        response.context_menu_opened()
    }

    fn apply_settings(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(window_level(self.settings)));
        #[cfg(windows)]
        apply_windows_opacity(self.window_handle, self.settings.opacity_percent);
        #[cfg(target_os = "macos")]
        macos_window::set_on_all_desktops(self.ns_view, self.settings.show_on_all_desktops);
    }
}

/// Segments in the eye outline. High enough that its straight edges fall less
/// than a hundredth of a point inside the curve they sample.
const EYE_SEGMENTS: usize = 128;
const EYE_OUTLINE_WIDTH: f32 = 4.0;

/// Shortens `offset` so the disc it centres still has the eye around it.
/// `pupil_offset` deliberately walks the iris right onto the lid at a full
/// squint, which would otherwise leave the centre outside the trimming bounds.
fn kept_inside(offset: Vec2, bounds: Vec2) -> Vec2 {
    let reach = (offset / bounds).length();
    if reach <= 0.999 {
        offset
    } else {
        offset * (0.999 / reach)
    }
}

fn ellipse_outline(center: Pos2, radius: Vec2) -> Vec<Pos2> {
    (0..EYE_SEGMENTS)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / EYE_SEGMENTS as f32;
            center + radius * Vec2::angled(angle)
        })
        .collect()
}

/// The part of a disc that falls inside the eye, as a convex polygon.
///
/// Scaling the eye to a unit circle turns the trim into a length test along the
/// ray from the disc's centre, which is where the eyelid would cut the iris.
/// Returns nothing when the disc has slid off the eye entirely.
fn disc_inside_ellipse(
    eye_center: Pos2,
    eye_radius: Vec2,
    disc_center: Pos2,
    disc_radius: f32,
) -> Vec<Pos2> {
    const SEGMENTS: usize = 64;

    let to_unit = |point: Pos2| (point - eye_center) / eye_radius;
    let from_unit = |unit: Vec2| eye_center + unit * eye_radius;

    let origin = to_unit(disc_center);
    if origin.length_sq() >= 1.0 {
        return Vec::new();
    }

    (0..SEGMENTS)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / SEGMENTS as f32;
            let edge = to_unit(disc_center + disc_radius * Vec2::angled(angle));
            from_unit(trim_to_unit_circle(origin, edge))
        })
        .collect()
}

/// Shortens the ray `origin -> edge` so that it ends on the unit circle.
/// `origin` must already lie inside it.
fn trim_to_unit_circle(origin: Vec2, edge: Vec2) -> Vec2 {
    if edge.length_sq() <= 1.0 {
        return edge;
    }

    let ray = edge - origin;
    let along = ray.length_sq();
    if along <= f32::EPSILON {
        return edge;
    }
    // |origin + t * ray| = 1, taking the root ahead of the origin.
    let offset = origin.dot(ray);
    let outside = origin.length_sq() - 1.0;
    let discriminant = (offset * offset - along * outside).max(0.0);
    let travel = ((-offset + discriminant.sqrt()) / along).clamp(0.0, 1.0);
    origin + ray * travel
}

fn shade(color: Color32, alpha: f32) -> Color32 {
    if alpha >= 1.0 {
        color
    } else {
        color.gamma_multiply(alpha)
    }
}

#[cfg(windows)]
impl Drop for GazeApp {
    fn drop(&mut self) {
        self.placement_tracker.flush();
    }
}

#[cfg(windows)]
fn windows_handle(
    creation_context: &eframe::CreationContext<'_>,
) -> Result<isize, Box<dyn std::error::Error + Send + Sync>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::Win32(handle) = creation_context.window_handle()?.as_raw() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Gaze requires a Win32 window handle",
        )
        .into());
    };
    Ok(handle.hwnd.get())
}

/// Fades the whole window with the layered-window alpha. The extended style is
/// only added while the window is actually translucent, so a fully opaque Gaze
/// keeps the exact rendering path it had before.
#[cfg(windows)]
fn apply_windows_opacity(window_handle: isize, opacity_percent: u8) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
        WS_EX_LAYERED,
    };

    let layered_bit = WS_EX_LAYERED as isize;
    // SAFETY: `window_handle` is the live main window created by eframe.
    unsafe {
        let style = GetWindowLongPtrW(window_handle as _, GWL_EXSTYLE);
        let is_layered = style & layered_bit != 0;
        if opacity_percent >= 100 {
            if is_layered {
                SetWindowLongPtrW(window_handle as _, GWL_EXSTYLE, style & !layered_bit);
            }
            return;
        }

        if !is_layered {
            SetWindowLongPtrW(window_handle as _, GWL_EXSTYLE, style | layered_bit);
        }
        let alpha = (f32::from(opacity_percent) * 2.55)
            .round()
            .clamp(0.0, 255.0) as u8;
        SetLayeredWindowAttributes(window_handle as _, 0, alpha, LWA_ALPHA);
    }
}

#[cfg(windows)]
struct FaceRegion {
    left_center: Pos2,
    right_center: Pos2,
    radius_x: f32,
    radius_y: f32,
    openness: f32,
    mouth: Option<(Pos2, Vec2)>,
}

#[cfg(windows)]
fn apply_windows_eye_region(window_handle: isize, pixels_per_point: f32, face: FaceRegion) {
    use windows_sys::Win32::Graphics::Gdi::{
        CombineRgn, CreateEllipticRgn, CreateRoundRectRgn, DeleteObject, RGN_OR, SetWindowRgn,
    };

    let FaceRegion {
        left_center,
        right_center,
        radius_x,
        radius_y,
        openness,
        mouth,
    } = face;
    let make_region = |center: Pos2| {
        let (region_x, region_y) = if openness < 0.075 || radius_y < 3.0 {
            (radius_x * 0.82 + 3.0, 3.5)
        } else {
            (radius_x + 2.5, radius_y + 2.5)
        };
        let left = ((center.x - region_x) * pixels_per_point).floor() as i32;
        let top = ((center.y - region_y) * pixels_per_point).floor() as i32;
        let right = ((center.x + region_x) * pixels_per_point).ceil() as i32;
        let bottom = ((center.y + region_y) * pixels_per_point).ceil() as i32;

        // SAFETY: all coordinates describe a small region within the app window.
        unsafe {
            if openness < 0.075 || radius_y < 3.0 {
                let roundness = (bottom - top).max(1);
                CreateRoundRectRgn(left, top, right, bottom, roundness, roundness)
            } else {
                CreateEllipticRgn(left, top, right, bottom)
            }
        }
    };

    let left_region = make_region(left_center);
    let right_region = make_region(right_center);
    if left_region.is_null() || right_region.is_null() {
        // SAFETY: any non-null handles here were created by GDI above.
        unsafe {
            if !left_region.is_null() {
                DeleteObject(left_region);
            }
            if !right_region.is_null() {
                DeleteObject(right_region);
            }
        }
        return;
    }

    // SAFETY: both region handles are live. On success SetWindowRgn transfers
    // ownership of `left_region` to Windows; `right_region` remains ours.
    unsafe {
        CombineRgn(left_region, left_region, right_region, RGN_OR);
        DeleteObject(right_region);
        if let Some((center, radius)) = mouth {
            let mouth_region = CreateEllipticRgn(
                ((center.x - radius.x - 2.5) * pixels_per_point).floor() as i32,
                ((center.y - radius.y - 2.5) * pixels_per_point).floor() as i32,
                ((center.x + radius.x + 2.5) * pixels_per_point).ceil() as i32,
                ((center.y + radius.y + 2.5) * pixels_per_point).ceil() as i32,
            );
            if !mouth_region.is_null() {
                CombineRgn(left_region, left_region, mouth_region, RGN_OR);
                DeleteObject(mouth_region);
            }
        }
        if SetWindowRgn(window_handle as _, left_region, 1) == 0 {
            DeleteObject(left_region);
        }
    }
}

impl eframe::App for GazeApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            if self.tray.handle_pending_action(ctx, &mut self.settings) {
                self.apply_settings(ctx);
                let _ = self.settings.save();
            }
            if let Some(placement) = current_window_placement(ctx) {
                self.placement_tracker.observe(placement);
            }
        }

        // Keep checking tray events even while the viewport is hidden.
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.paint(ui, &ctx);
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        #[cfg(windows)]
        return OUTLINE.to_normalized_gamma_f32();

        #[cfg(not(windows))]
        [0.0, 0.0, 0.0, 0.0]
    }
}

#[cfg(windows)]
fn current_window_placement(ctx: &egui::Context) -> Option<placement::WindowPlacement> {
    ctx.input(|input| {
        let viewport = input.viewport();
        let outer = viewport.outer_rect?;
        let inner = viewport.inner_rect?;
        let width = inner.width().round().max(1.0) as u32;
        let height = inner.height().round().max(1.0) as u32;
        Some(placement::WindowPlacement {
            x: outer.left().round() as i32,
            y: outer.top().round() as i32,
            width,
            height,
        })
    })
}

#[cfg(windows)]
const fn placement_default_width() -> f32 {
    placement::DEFAULT_WIDTH
}

#[cfg(not(windows))]
const fn placement_default_width() -> f32 {
    380.0
}

#[cfg(windows)]
const fn placement_default_height() -> f32 {
    placement::DEFAULT_HEIGHT
}

#[cfg(not(windows))]
const fn placement_default_height() -> f32 {
    220.0
}

#[cfg(windows)]
const fn placement_min_width() -> f32 {
    placement::MIN_WIDTH
}

#[cfg(not(windows))]
const fn placement_min_width() -> f32 {
    120.0
}

#[cfg(windows)]
const fn placement_min_height() -> f32 {
    placement::MIN_HEIGHT
}

#[cfg(not(windows))]
const fn placement_min_height() -> f32 {
    80.0
}

#[cfg(windows)]
const fn placement_max_width() -> f32 {
    placement::MAX_WIDTH
}

#[cfg(not(windows))]
const fn placement_max_width() -> f32 {
    540.0
}

#[cfg(windows)]
const fn placement_max_height() -> f32 {
    placement::MAX_HEIGHT
}

#[cfg(not(windows))]
const fn placement_max_height() -> f32 {
    320.0
}

#[cfg(windows)]
fn windows_cursor_in_viewport(ctx: &egui::Context) -> Option<Pos2> {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid writable POINT for the duration of this call.
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return None;
    }

    // Read this before `Context::input`: both APIs acquire the same egui lock,
    // so nesting `pixels_per_point` inside the closure deadlocks in debug builds.
    let pixels_per_point = ctx.pixels_per_point();
    ctx.input(|input| {
        let viewport = input.viewport();
        let window_origin = viewport.inner_rect?.min;
        Some(Pos2::new(
            point.x as f32 / pixels_per_point - window_origin.x,
            point.y as f32 / pixels_per_point - window_origin.y,
        ))
    })
}

/// The AppKit view backing the window, or `0` when it cannot be identified.
/// Only cosmetic window behaviour depends on it, so a miss is not fatal.
#[cfg(target_os = "macos")]
fn appkit_view(creation_context: &eframe::CreationContext<'_>) -> usize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match creation_context
        .window_handle()
        .map(|handle| handle.as_raw())
    {
        Ok(RawWindowHandle::AppKit(handle)) => handle.ns_view.as_ptr() as usize,
        _ => 0,
    }
}

#[cfg(target_os = "macos")]
fn desktop_cursor_in_viewport(ctx: &egui::Context) -> Option<Pos2> {
    let cursor = macos_cursor::desktop_position()?;
    ctx.input(|input| {
        let window_origin = input.viewport().inner_rect?.min;
        Some(cursor - window_origin.to_vec2())
    })
}

#[cfg(not(windows))]
fn keyboard_activity_marker(ctx: &egui::Context, elapsed: Duration) -> Option<u32> {
    ctx.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
            .then_some(elapsed.as_millis() as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eye_radii(width: f32, height: f32) -> (f32, f32) {
        (eye_radius_x(width), eye_radius_y(height))
    }

    /// Every window size the user can drag Gaze to, plus the corners between.
    fn window_sizes() -> Vec<(f32, f32)> {
        let widths = [
            placement_min_width(),
            160.0,
            placement_default_width(),
            placement_max_width(),
        ];
        let heights = [
            placement_min_height(),
            110.0,
            placement_default_height(),
            placement_max_height(),
        ];
        widths
            .iter()
            .flat_map(|&width| heights.iter().map(move |&height| (width, height)))
            .collect()
    }

    #[test]
    fn the_face_fits_inside_the_smallest_window_gaze_allows() {
        // The eyes and their outlines are laid out from the window's width, so
        // shrinking the window has to shrink them rather than crop them.
        for (width, height) in window_sizes() {
            let (radius_x, radius_y) = eye_radii(width, height);

            // Two eyes, the gap between them, and the outline around the pair.
            let face_width = 4.0 * radius_x + 2.0 * EYE_GAP + EYE_OUTLINE_WIDTH;
            let face_height = 2.0 * radius_y + EYE_OUTLINE_WIDTH;
            assert!(face_width <= width, "{width}x{height}: {face_width} wide");
            assert!(
                face_height <= height,
                "{width}x{height}: {face_height} tall"
            );
        }
    }

    #[test]
    fn a_small_face_keeps_room_to_drag_between_its_resize_edges() {
        for (width, height) in window_sizes() {
            let (radius_x, radius_y) = eye_radii(width, height);
            let bounds = Rect::from_center_size(
                Pos2::new(width / 2.0, height / 2.0),
                Vec2::new(
                    4.0 * radius_x + 2.0 * EYE_GAP + 2.0 * EYE_GAP,
                    2.0 * radius_y + 2.0 * EYE_GAP,
                ),
            );

            let edge = resize_edge(bounds);
            assert!(
                bounds.height() > 3.0 * edge,
                "{width}x{height}: the resize bands leave nothing to drag"
            );
        }
    }

    fn outside_the_eye(eye_center: Pos2, eye_radius: Vec2, point: Pos2) -> f32 {
        ((point - eye_center) / eye_radius).length()
    }

    #[test]
    fn an_iris_looking_into_a_corner_stays_inside_the_eye() {
        // The case a rectangular clip between the lids used to let escape.
        let eye_center = Pos2::new(0.0, 0.0);
        let eye_radius = Vec2::new(85.5, 29.3);
        let iris_center = Pos2::new(-40.0, 18.0);

        let outline = disc_inside_ellipse(eye_center, eye_radius, iris_center, 30.8);
        assert!(!outline.is_empty());
        for point in outline {
            assert!(
                outside_the_eye(eye_center, eye_radius, point) <= 1.001,
                "{point:?} escaped the eye"
            );
        }
    }

    #[test]
    fn nothing_is_ever_painted_past_the_eyelid() {
        // Every window size, every squint, every direction the eyes can look:
        // a translucent Gaze shows anything that reaches the lid straight
        // through it, so nothing may be drawn beyond the outline's inner edge.
        let eye_center = Pos2::new(0.0, 0.0);
        for width in [
            placement_min_width(),
            placement_default_width(),
            placement_max_width(),
        ] {
            for height in [
                placement_min_height(),
                placement_default_height(),
                placement_max_height(),
            ] {
                let (radius_x, base_radius_y) = eye_radii(width, height);
                let iris = iris_radius(radius_x, base_radius_y);

                for squint_step in 0..=10 {
                    let narrowing = squint_step as f32 / 10.0;
                    let radius_y = (base_radius_y * (1.0 - 0.62 * narrowing)).max(3.0);
                    let eye_radius = Vec2::new(radius_x, radius_y);
                    let bounds =
                        (eye_radius - Vec2::splat(EYE_OUTLINE_WIDTH / 2.0)).max(Vec2::splat(0.5));

                    for angle_step in 0..36 {
                        let angle = std::f32::consts::TAU * angle_step as f32 / 36.0;
                        let offset = pupil_offset(
                            10_000.0 * Vec2::angled(angle),
                            radius_x,
                            radius_y,
                            iris,
                            narrowing,
                        );
                        let iris_center = eye_center + kept_inside(offset, bounds);

                        for radius in [iris, iris * 0.54] {
                            let outline =
                                disc_inside_ellipse(eye_center, bounds, iris_center, radius);
                            assert!(
                                !outline.is_empty(),
                                "{width}x{height} squint {narrowing} angle {angle}: iris vanished"
                            );
                            for point in outline {
                                assert!(
                                    outside_the_eye(eye_center, bounds, point) <= 1.001,
                                    "{width}x{height} squint {narrowing} angle {angle}: \
                                     {point:?} escaped past the eyelid"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn an_iris_well_inside_the_eye_keeps_its_round_shape() {
        let eye_center = Pos2::new(0.0, 0.0);
        let eye_radius = Vec2::new(85.5, 77.0);
        let iris_center = Pos2::new(6.0, -4.0);
        let iris = 30.8;

        for point in disc_inside_ellipse(eye_center, eye_radius, iris_center, iris) {
            assert!(
                (point.distance(iris_center) - iris).abs() < 0.01,
                "{point:?} was trimmed although the iris fits"
            );
        }
    }

    #[test]
    fn an_iris_driven_off_the_eye_draws_nothing() {
        let outline = disc_inside_ellipse(
            Pos2::new(0.0, 0.0),
            Vec2::new(85.5, 29.3),
            Pos2::new(0.0, 40.0),
            30.8,
        );
        assert!(outline.is_empty());
    }

    #[test]
    fn both_eye_axes_move_the_iris_size() {
        let (radius_x, radius_y) = eye_radii(380.0, 220.0);
        let default = iris_radius(radius_x, radius_y);

        let (narrow_x, narrow_y) = eye_radii(260.0, 220.0);
        let (short_x, short_y) = eye_radii(380.0, 150.0);
        assert!(
            iris_radius(narrow_x, narrow_y) < default,
            "narrowing shrinks"
        );
        assert!(
            iris_radius(short_x, short_y) < default,
            "flattening shrinks"
        );

        let (big_x, big_y) = eye_radii(540.0, 320.0);
        assert!(iris_radius(big_x, big_y) > default);
    }

    #[test]
    fn the_iris_stays_visible_and_inside_the_eye_at_every_window_size() {
        for width in [
            placement_min_width(),
            placement_default_width(),
            placement_max_width(),
        ] {
            for height in [
                placement_min_height(),
                placement_default_height(),
                placement_max_height(),
            ] {
                let (radius_x, radius_y) = eye_radii(width, height);
                let iris = iris_radius(radius_x, radius_y);
                assert!(iris >= IRIS_MIN_RADIUS, "{width}x{height}: {iris}");
                assert!(iris < radius_y, "{width}x{height}: {iris} vs {radius_y}");
                assert!(iris < radius_x, "{width}x{height}: {iris} vs {radius_x}");
            }
        }
    }
}
