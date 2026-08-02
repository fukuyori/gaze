#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod animation;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod native_resize;
#[cfg(windows)]
mod placement;
mod repaint_watchdog;
#[cfg(windows)]
mod tray;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use animation::{Blinker, IdleAnimator, distance_squint, pupil_offset};
use eframe::egui::{
    self, Color32, CursorIcon, PointerButton, Pos2, Rect, ResizeDirection, Shape, Stroke, Vec2,
    ViewportBuilder, ViewportCommand,
};

const OUTLINE: Color32 = Color32::from_rgb(39, 38, 42);
const SCLERA: Color32 = Color32::from_rgb(255, 254, 250);
const IRIS: Color32 = Color32::from_rgb(73, 137, 154);
const PUPIL: Color32 = Color32::from_rgb(24, 31, 36);
const MOUTH: Color32 = Color32::from_rgb(68, 39, 46);
const TONGUE: Color32 = Color32::from_rgb(205, 104, 119);

#[derive(Clone, Copy)]
struct EyelidState {
    openness: f32,
    narrowing: f32,
}

fn main() -> eframe::Result {
    #[cfg(windows)]
    let restored_placement = placement::WindowPlacement::load().ok().flatten();
    #[cfg(windows)]
    let centered = restored_placement.is_none();
    #[cfg(not(windows))]
    let centered = true;

    let mut viewport = ViewportBuilder::default()
        .with_title("Gaze")
        .with_inner_size([placement_default_width(), placement_default_height()])
        .with_min_inner_size([placement_min_width(), placement_min_height()])
        .with_max_inner_size([placement_max_width(), placement_max_height()])
        .with_resizable(true)
        .with_transparent(true)
        .with_decorations(false)
        .with_taskbar(false)
        .with_has_shadow(false);

    #[cfg(windows)]
    if let Some(restored) = restored_placement {
        viewport = viewport
            .with_position([restored.x as f32, restored.y as f32])
            .with_inner_size(restored.restored_size());
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered,
        ..Default::default()
    };

    eframe::run_native(
        "Gaze",
        options,
        Box::new(|creation_context| {
            GazeApp::new(creation_context).map(|app| Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}

struct GazeApp {
    started_at: Instant,
    blinker: Blinker,
    idle_animator: IdleAnimator,
    last_cursor: Option<Pos2>,
    _repaint_watchdog: repaint_watchdog::RepaintWatchdog,
    #[cfg(windows)]
    tray: tray::TrayState,
    #[cfg(windows)]
    window_handle: isize,
    #[cfg(windows)]
    placement_tracker: placement::PlacementTracker,
    #[cfg(windows)]
    native_resize: native_resize::NativeResize,
}

impl GazeApp {
    fn new(
        creation_context: &eframe::CreationContext<'_>,
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
        let tray = tray::TrayState::new(&creation_context.egui_ctx, placement_tracker.shared())?;
        Ok(Self {
            started_at: Instant::now(),
            blinker: Blinker::new(seed),
            idle_animator: IdleAnimator::new(),
            last_cursor: None,
            _repaint_watchdog: repaint_watchdog::RepaintWatchdog::new(&creation_context.egui_ctx)?,
            #[cfg(windows)]
            tray,
            #[cfg(windows)]
            window_handle,
            #[cfg(windows)]
            placement_tracker,
            #[cfg(windows)]
            native_resize: native_resize::NativeResize::new(window_handle)?,
        })
    }

    fn cursor_in_viewport(&mut self, ctx: &egui::Context) -> Pos2 {
        #[cfg(windows)]
        if let Some(cursor) = windows_cursor_in_viewport(ctx) {
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
        let monitor_diagonal = ctx.input(|input| {
            input
                .viewport()
                .monitor_size
                .unwrap_or(rect.size())
                .length()
        });
        let distance = (cursor - rect.center()).length();
        let squint = distance_squint(distance, monitor_diagonal);
        let elapsed = self.started_at.elapsed();
        let idle = self.idle_animator.expression(elapsed, cursor);
        let blink = self.blinker.closure(elapsed);
        let openness =
            ((1.0 - 0.62 * squint) * (1.0 - blink) * (1.0 - 0.18 * idle.yawn) * (1.0 - idle.sleep))
                .clamp(0.0, 1.0);

        let scale = (rect.height() / 220.0).clamp(0.68, 1.46);
        let radius_x = ((rect.width() - 38.0) / 4.0).clamp(46.0, 125.0);
        let base_radius_y = (rect.height() * 0.35).clamp(38.0, 110.0);
        let radius_y = base_radius_y * openness;
        // Leave room for the 4-point outlines so the two eyes do not overlap.
        // Pupil travel includes the same half-gap and can still reach center.
        let spacing = radius_x + 3.0;
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
            Pos2::new(left.x - radius_x - 3.0, eye_center.y - radius_y - 3.0),
            Pos2::new(right.x + radius_x + 3.0, eye_center.y + radius_y + 3.0),
        );
        if let Some((mouth_center, mouth_radius)) = mouth {
            face_bounds =
                face_bounds.union(Rect::from_center_size(mouth_center, 2.0 * mouth_radius));
        }
        #[cfg(windows)]
        self.native_resize
            .update(face_bounds, ctx.pixels_per_point(), openness > 0.15);
        self.handle_window_interaction(ui, ctx, face_bounds, openness);

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
        let eyelids = EyelidState {
            openness,
            narrowing: squint,
        };
        self.paint_eye(painter, left, cursor, radius_x, radius_y, eyelids);
        self.paint_eye(painter, right, cursor, radius_x, radius_y, eyelids);
        if let Some((center, radius)) = mouth {
            self.paint_mouth(painter, center, radius, idle.yawn);
        }
    }

    fn paint_eye(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        cursor: Pos2,
        radius_x: f32,
        radius_y: f32,
        eyelids: EyelidState,
    ) {
        if eyelids.openness < 0.075 || radius_y < 3.0 {
            let half_width = radius_x * 0.82;
            painter.line_segment(
                [
                    center - Vec2::new(half_width, 0.0),
                    center + Vec2::new(half_width, 0.0),
                ],
                Stroke::new(5.0, OUTLINE),
            );
            return;
        }

        let eye_radius = Vec2::new(radius_x, radius_y.max(3.0));
        painter.add(Shape::ellipse_filled(center, eye_radius, SCLERA));

        // Distance squinting changes the eyelid opening, not the iris size.
        // The iris is allowed to move under the upper/lower lid and is clipped
        // to the current vertical eye opening below.
        let iris_radius = (radius_x * 0.38).clamp(2.0, 44.0);
        let offset = pupil_offset(
            cursor - center,
            radius_x,
            radius_y,
            iris_radius,
            eyelids.narrowing,
        );
        let iris_center = center + offset;
        let eye_clip = Rect::from_min_max(
            Pos2::new(painter.clip_rect().left(), center.y - radius_y),
            Pos2::new(painter.clip_rect().right(), center.y + radius_y),
        );
        let iris_painter = painter.with_clip_rect(eye_clip);

        iris_painter.circle_filled(iris_center, iris_radius, IRIS);
        iris_painter.circle_filled(iris_center, iris_radius * 0.54, PUPIL);
        if iris_radius > 7.0 {
            iris_painter.circle_filled(
                iris_center - Vec2::splat(iris_radius * 0.27),
                (iris_radius * 0.16).max(1.5),
                Color32::WHITE,
            );
        }

        painter.add(Shape::ellipse_stroke(
            center,
            eye_radius,
            Stroke::new(4.0, OUTLINE),
        ));
    }

    fn paint_mouth(&self, painter: &egui::Painter, center: Pos2, radius: Vec2, openness: f32) {
        let scale = radius.x / 30.0;
        painter.add(Shape::ellipse_filled(center, radius, OUTLINE));
        let inner_radius = Vec2::new(
            radius.x - 5.0 * scale,
            (radius.y - 5.0 * scale).max(1.5 * scale),
        );
        painter.add(Shape::ellipse_filled(center, inner_radius, MOUTH));

        if openness > 0.38 {
            let tongue_radius = Vec2::new(12.0, 2.5 + 2.5 * openness) * scale;
            let tongue_center = center + Vec2::new(0.0, inner_radius.y * 0.48);
            painter.add(Shape::ellipse_filled(tongue_center, tongue_radius, TONGUE));
        }
    }

    fn handle_window_interaction(
        &self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        face_bounds: Rect,
        openness: f32,
    ) {
        let response = ui.interact(
            ui.max_rect(),
            ui.id().with("window-drag-or-resize"),
            egui::Sense::drag(),
        );
        let direction = if openness > 0.15 {
            ctx.pointer_hover_pos().and_then(|pointer| {
                let edge = 10.0;
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

        if let Some(direction) = direction {
            ctx.set_cursor_icon(match direction {
                ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
                ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
                _ => CursorIcon::Default,
            });
            #[cfg(not(windows))]
            if response.hovered()
                && ctx.input(|input| input.pointer.button_pressed(PointerButton::Primary))
            {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
            }
        } else if response.hovered()
            && ctx.input(|input| input.pointer.button_pressed(PointerButton::Primary))
        {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
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
            self.tray.handle_pending_action(ctx);
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
    260.0
}

#[cfg(windows)]
const fn placement_min_height() -> f32 {
    placement::MIN_HEIGHT
}

#[cfg(not(windows))]
const fn placement_min_height() -> f32 {
    150.0
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
