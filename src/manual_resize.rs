//! Edge resizing for macOS, where winit cannot start a native resize drag.
//!
//! `ViewportCommand::BeginResize` is a no-op on macOS (winit returns
//! `NotSupported`), so a borderless window has to move and resize itself while
//! the pointer is dragged.
//!
//! Every frame resolves the window rectangle from the rectangle the drag
//! started with and the current *desktop* cursor position, never from the
//! window's live geometry. Deriving the next rectangle from the current one
//! feeds the window's own movement back into the calculation: moving the left
//! edge shifts the window, which shifts every window-relative pointer reading,
//! which moves the edge again, and the window slides away on its own.

use eframe::egui::{Pos2, Rect, ResizeDirection, Vec2};

#[derive(Clone, Copy, Debug)]
pub struct ResizeDrag {
    direction: ResizeDirection,
    /// Window rectangle when the drag started; the edges away from `direction`
    /// stay exactly where they were.
    start: Rect,
    /// Distance from the pointer to the dragged edge when the drag started.
    grab: f32,
}

impl ResizeDrag {
    pub fn start(direction: ResizeDirection, window: Rect, pointer: Pos2) -> Self {
        let grab = match direction {
            ResizeDirection::West => window.left() - pointer.x,
            ResizeDirection::East => window.right() - pointer.x,
            ResizeDirection::North => window.top() - pointer.y,
            _ => window.bottom() - pointer.y,
        };
        Self {
            direction,
            start: window,
            grab,
        }
    }

    /// The window rectangle this drag asks for, clamped to the size limits.
    pub fn resolve(self, pointer: Pos2, min: Vec2, max: Vec2) -> Rect {
        let start = self.start;
        let edge = match self.direction {
            ResizeDirection::West | ResizeDirection::East => pointer.x + self.grab,
            _ => pointer.y + self.grab,
        };

        match self.direction {
            ResizeDirection::West => {
                let width = (start.right() - edge).clamp(min.x, max.x);
                Rect::from_min_size(
                    Pos2::new(start.right() - width, start.top()),
                    Vec2::new(width, start.height()),
                )
            }
            ResizeDirection::East => {
                let width = (edge - start.left()).clamp(min.x, max.x);
                Rect::from_min_size(start.min, Vec2::new(width, start.height()))
            }
            ResizeDirection::North => {
                let height = (start.bottom() - edge).clamp(min.y, max.y);
                Rect::from_min_size(
                    Pos2::new(start.left(), start.bottom() - height),
                    Vec2::new(start.width(), height),
                )
            }
            _ => {
                let height = (edge - start.top()).clamp(min.y, max.y);
                Rect::from_min_size(start.min, Vec2::new(start.width(), height))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Vec2 = Vec2::new(260.0, 150.0);
    const MAX: Vec2 = Vec2::new(540.0, 320.0);

    fn window() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 60.0), Vec2::new(380.0, 220.0))
    }

    #[test]
    fn dragging_the_right_edge_only_changes_the_width() {
        let outer = window();
        let grab = Pos2::new(474.0, 170.0);
        let drag = ResizeDrag::start(ResizeDirection::East, outer, grab);
        let resized = drag.resolve(grab + Vec2::new(40.0, 0.0), MIN, MAX);
        assert_eq!(resized.min, outer.min);
        assert_eq!(resized.size(), Vec2::new(420.0, 220.0));
    }

    #[test]
    fn dragging_the_left_edge_pins_the_right_edge() {
        let outer = window();
        let grab = Pos2::new(104.0, 170.0);
        let drag = ResizeDrag::start(ResizeDirection::West, outer, grab);
        let resized = drag.resolve(grab - Vec2::new(40.0, 0.0), MIN, MAX);
        assert_eq!(resized.right(), outer.right());
        assert_eq!(resized.top(), outer.top());
        assert_eq!(resized.size(), Vec2::new(420.0, 220.0));
    }

    #[test]
    fn dragging_the_top_edge_pins_the_bottom_edge() {
        let outer = window();
        let grab = Pos2::new(280.0, 64.0);
        let drag = ResizeDrag::start(ResizeDirection::North, outer, grab);
        let resized = drag.resolve(grab - Vec2::new(0.0, 30.0), MIN, MAX);
        assert_eq!(resized.bottom(), outer.bottom());
        assert_eq!(resized.left(), outer.left());
        assert_eq!(resized.size(), Vec2::new(380.0, 250.0));
    }

    #[test]
    fn dragging_the_bottom_edge_only_changes_the_height() {
        let outer = window();
        let grab = Pos2::new(280.0, 276.0);
        let drag = ResizeDrag::start(ResizeDirection::South, outer, grab);
        let resized = drag.resolve(grab + Vec2::new(0.0, 30.0), MIN, MAX);
        assert_eq!(resized.min, outer.min);
        assert_eq!(resized.size(), Vec2::new(380.0, 250.0));
    }

    #[test]
    fn a_still_pointer_holds_the_window_still() {
        // Re-resolving the same pointer must never walk the window along, no
        // matter how many frames the drag lasts.
        let grab = Pos2::new(104.0, 170.0);
        let pointer = grab - Vec2::new(40.0, 0.0);
        let drag = ResizeDrag::start(ResizeDirection::West, window(), grab);
        let first = drag.resolve(pointer, MIN, MAX);
        for _ in 0..10 {
            assert_eq!(drag.resolve(pointer, MIN, MAX), first);
        }
    }

    #[test]
    fn a_clamped_drag_releases_from_the_same_place_it_grabbed() {
        // Pushing past the minimum and coming back must land on the original
        // size rather than an offset one.
        let outer = window();
        let grab = Pos2::new(104.0, 170.0);
        let drag = ResizeDrag::start(ResizeDirection::West, outer, grab);
        drag.resolve(grab + Vec2::new(400.0, 0.0), MIN, MAX);
        assert_eq!(drag.resolve(grab, MIN, MAX), outer);
    }

    #[test]
    fn size_limits_stop_the_drag() {
        let outer = window();
        let grab = Pos2::new(474.0, 170.0);
        let drag = ResizeDrag::start(ResizeDirection::East, outer, grab);
        assert_eq!(
            drag.resolve(grab + Vec2::new(400.0, 0.0), MIN, MAX).width(),
            MAX.x
        );
        assert_eq!(
            drag.resolve(grab - Vec2::new(400.0, 0.0), MIN, MAX).width(),
            MIN.x
        );
    }
}
