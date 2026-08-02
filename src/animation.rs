use std::time::Duration;

const BLINK_CLOSE: f32 = 0.075;
const BLINK_HOLD: f32 = 0.045;
const BLINK_OPEN: f32 = 0.13;
const YAWN_START: f32 = 7.0;
const YAWN_OPEN: f32 = 0.9;
const YAWN_HOLD: f32 = 1.3;
const YAWN_CLOSE: f32 = 1.0;
const SLEEP_START: f32 = 15.0;
const SLEEP_CLOSE: f32 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlinkPhase {
    Waiting { until: f32 },
    Blinking { started_at: f32 },
}

/// Produces irregular, smooth blinks without depending on a random-number crate.
pub struct Blinker {
    phase: BlinkPhase,
    random_state: u64,
}

impl Blinker {
    pub fn new(seed: u64) -> Self {
        let mut blinker = Self {
            phase: BlinkPhase::Waiting { until: 0.0 },
            random_state: seed.max(1),
        };
        let delay = blinker.next_delay(1.2, 3.2);
        blinker.phase = BlinkPhase::Waiting { until: delay };
        blinker
    }

    /// Returns eyelid closure from `0.0` (open) through `1.0` (closed).
    pub fn closure(&mut self, elapsed: Duration) -> f32 {
        let now = elapsed.as_secs_f32();

        if let BlinkPhase::Waiting { until } = self.phase {
            if now < until {
                return 0.0;
            }
            self.phase = BlinkPhase::Blinking { started_at: now };
        }

        let BlinkPhase::Blinking { started_at } = self.phase else {
            return 0.0;
        };
        let blink_time = now - started_at;
        let total = BLINK_CLOSE + BLINK_HOLD + BLINK_OPEN;

        if blink_time >= total {
            let delay = self.next_delay(2.3, 6.4);
            self.phase = BlinkPhase::Waiting { until: now + delay };
            return 0.0;
        }

        if blink_time < BLINK_CLOSE {
            smoothstep(blink_time / BLINK_CLOSE)
        } else if blink_time < BLINK_CLOSE + BLINK_HOLD {
            1.0
        } else {
            let opening = (blink_time - BLINK_CLOSE - BLINK_HOLD) / BLINK_OPEN;
            1.0 - smoothstep(opening)
        }
    }

    fn next_delay(&mut self, min: f32, max: f32) -> f32 {
        // xorshift64*: sufficient here because it only varies animation timing.
        let mut value = self.random_state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.random_state = value;
        let sample = value.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let unit = (sample >> 40) as f32 / ((1_u32 << 24) - 1) as f32;
        min + (max - min) * unit
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct IdleExpression {
    /// Mouth opening from `0.0` through `1.0`.
    pub yawn: f32,
    /// Eye closure caused by sleep from `0.0` through `1.0`.
    pub sleep: f32,
}

/// Turns pointer inactivity into a yawn followed by sleep.
pub struct IdleAnimator {
    last_cursor: Option<eframe::egui::Pos2>,
    last_input_marker: Option<u32>,
    last_active_at: f32,
}

impl IdleAnimator {
    pub fn new() -> Self {
        Self {
            last_cursor: None,
            last_input_marker: None,
            last_active_at: 0.0,
        }
    }

    pub fn expression(
        &mut self,
        elapsed: Duration,
        cursor: eframe::egui::Pos2,
        input_marker: Option<u32>,
    ) -> IdleExpression {
        let now = elapsed.as_secs_f32();
        let moved = self
            .last_cursor
            .is_some_and(|previous| previous.distance(cursor) >= 1.0);
        let input_changed = input_marker.is_some_and(|marker| {
            let changed = self.last_input_marker != Some(marker);
            self.last_input_marker = Some(marker);
            changed
        });

        if self.last_cursor.is_none() || moved || input_changed {
            self.last_cursor = Some(cursor);
            self.last_active_at = now;
            return IdleExpression::default();
        }

        let idle = (now - self.last_active_at).max(0.0);
        let yawn_time = idle - YAWN_START;
        let yawn = if yawn_time < 0.0 {
            0.0
        } else if yawn_time < YAWN_OPEN {
            smoothstep(yawn_time / YAWN_OPEN)
        } else if yawn_time < YAWN_OPEN + YAWN_HOLD {
            1.0
        } else if yawn_time < YAWN_OPEN + YAWN_HOLD + YAWN_CLOSE {
            1.0 - smoothstep((yawn_time - YAWN_OPEN - YAWN_HOLD) / YAWN_CLOSE)
        } else {
            0.0
        };

        IdleExpression {
            yawn,
            sleep: smoothstep((idle - SLEEP_START) / SLEEP_CLOSE),
        }
    }
}

/// How strongly the eyes squint for a cursor at `distance`.
pub fn distance_squint(distance: f32, monitor_diagonal: f32) -> f32 {
    let near = (monitor_diagonal * 0.24).max(360.0);
    let far = (monitor_diagonal * 0.72).max(near + 1.0);
    smoothstep(((distance - near) / (far - near)).clamp(0.0, 1.0))
}

/// Pupil travel along a direction, constrained to the current eye opening.
pub fn pupil_offset(
    cursor_delta: eframe::egui::Vec2,
    radius_x: f32,
    radius_y: f32,
    iris_radius: f32,
    eyelid_narrowing: f32,
) -> eframe::egui::Vec2 {
    let distance = cursor_delta.length();
    if distance <= f32::EPSILON {
        return eframe::egui::Vec2::ZERO;
    }

    let direction = cursor_delta / distance;
    // Scale the response distance with the eye width. A fixed 180-point range
    // barely moved small eyes when the cursor was between them, so the inward
    // directions were correct but did not read visually as convergence.
    let full_travel_distance = (radius_x + 3.0).max(1.0);
    let travel = smoothstep((distance / full_travel_distance).clamp(0.0, 1.0));
    let room_x = (radius_x + 3.0 - iris_radius).max(0.0);
    // At a fully open eye the iris only reaches the lid. As the lid narrows,
    // allow it to cover progressively more of the iris, reaching exactly half
    // at maximum distance squinting.
    let eyelid_narrowing = eyelid_narrowing.clamp(0.0, 1.0);
    let room_y = (radius_y - iris_radius * (1.0 - eyelid_narrowing)).max(0.0);

    eframe::egui::vec2(direction.x * room_x, direction.y * room_y) * travel
}

fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_cursor_does_not_squint() {
        assert_eq!(distance_squint(100.0, 2_000.0), 0.0);
    }

    #[test]
    fn remote_cursor_reaches_full_squint() {
        assert_eq!(distance_squint(2_000.0, 2_000.0), 1.0);
    }

    #[test]
    fn pupil_stays_within_available_eye_area() {
        let offset = pupil_offset(eframe::egui::vec2(10_000.0, 5_000.0), 70.0, 50.0, 18.0, 0.0);
        assert!(offset.x.abs() <= 55.0);
        assert!(offset.y.abs() <= 32.0);
    }

    #[test]
    fn open_eye_does_not_hide_remote_upper_iris() {
        let radius_y = 50.0;
        let iris_radius = 18.0;
        let offset = pupil_offset(
            eframe::egui::vec2(0.0, -10_000.0),
            70.0,
            radius_y,
            iris_radius,
            0.0,
        );

        let iris_top = offset.y - iris_radius;
        assert!((iris_top + radius_y).abs() < 0.001);
    }

    #[test]
    fn maximum_squint_hides_half_of_remote_upper_iris() {
        let radius_y = 29.0;
        let iris_radius = 24.0;
        let offset = pupil_offset(
            eframe::egui::vec2(0.0, -10_000.0),
            70.0,
            radius_y,
            iris_radius,
            1.0,
        );

        assert!((offset.y + radius_y).abs() < 0.001);
    }

    #[test]
    fn maximum_squint_hides_half_of_remote_lower_iris() {
        let radius_y = 29.0;
        let iris_radius = 24.0;
        let offset = pupil_offset(
            eframe::egui::vec2(0.0, 10_000.0),
            70.0,
            radius_y,
            iris_radius,
            1.0,
        );

        assert!((offset.y - radius_y).abs() < 0.001);
    }

    #[test]
    fn irises_touch_when_cursor_is_between_the_eyes() {
        let radius_x = 55.5;
        let radius_y = 52.5;
        let iris_radius = radius_x * 0.38;
        let spacing = radius_x + 3.0;
        let left_offset = pupil_offset(
            eframe::egui::vec2(spacing, 0.0),
            radius_x,
            radius_y,
            iris_radius,
            0.0,
        );
        let right_offset = pupil_offset(
            eframe::egui::vec2(-spacing, 0.0),
            radius_x,
            radius_y,
            iris_radius,
            0.0,
        );

        let left_iris_inner_edge = -spacing + left_offset.x + iris_radius;
        let right_iris_inner_edge = spacing + right_offset.x - iris_radius;
        assert!(spacing > radius_x + 2.0);
        assert!((left_iris_inner_edge - right_iris_inner_edge).abs() < 0.001);
        assert!(left_iris_inner_edge.abs() < 0.001);
        assert_eq!(left_offset.x, -right_offset.x);
    }

    #[test]
    fn blink_closes_and_opens() {
        let mut blinker = Blinker::new(1);
        blinker.phase = BlinkPhase::Waiting { until: 0.0 };

        assert_eq!(blinker.closure(Duration::ZERO), 0.0);
        assert!(blinker.closure(Duration::from_millis(60)) > 0.8);
        assert_eq!(blinker.closure(Duration::from_millis(90)), 1.0);
        assert!(blinker.closure(Duration::from_millis(200)) < 0.7);
        assert_eq!(blinker.closure(Duration::from_millis(300)), 0.0);
    }

    #[test]
    fn idle_pointer_yawns_then_sleeps() {
        let cursor = eframe::egui::pos2(100.0, 100.0);
        let mut idle = IdleAnimator::new();

        assert_eq!(
            idle.expression(Duration::ZERO, cursor, None),
            IdleExpression::default()
        );
        assert_eq!(
            idle.expression(Duration::from_secs_f32(6.9), cursor, None),
            IdleExpression::default()
        );
        assert!(
            idle.expression(Duration::from_secs_f32(7.8), cursor, None)
                .yawn
                > 0.9
        );
        assert!(
            idle.expression(Duration::from_secs_f32(16.2), cursor, None)
                .sleep
                > 0.99
        );
    }

    #[test]
    fn pointer_movement_wakes_immediately() {
        let mut idle = IdleAnimator::new();
        let cursor = eframe::egui::pos2(100.0, 100.0);
        idle.expression(Duration::ZERO, cursor, None);
        assert!(
            idle.expression(Duration::from_secs_f32(16.2), cursor, None)
                .sleep
                > 0.99
        );

        assert_eq!(
            idle.expression(
                Duration::from_secs_f32(16.3),
                cursor + eframe::egui::vec2(2.0, 0.0),
                None,
            ),
            IdleExpression::default()
        );
    }

    #[test]
    fn keyboard_input_prevents_idle_expression_while_mouse_is_still() {
        let mut idle = IdleAnimator::new();
        let cursor = eframe::egui::pos2(100.0, 100.0);
        idle.expression(Duration::ZERO, cursor, Some(100));
        assert!(
            idle.expression(Duration::from_secs_f32(16.2), cursor, Some(100))
                .sleep
                > 0.99
        );

        assert_eq!(
            idle.expression(Duration::from_secs_f32(16.3), cursor, Some(101)),
            IdleExpression::default()
        );
        assert_eq!(
            idle.expression(Duration::from_secs_f32(23.2), cursor, Some(101)),
            IdleExpression::default()
        );
        assert!(
            idle.expression(Duration::from_secs_f32(24.1), cursor, Some(101))
                .yawn
                > 0.9
        );
    }
}
