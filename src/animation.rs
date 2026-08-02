use std::time::Duration;

const BLINK_CLOSE: f32 = 0.075;
const BLINK_HOLD: f32 = 0.045;
const BLINK_OPEN: f32 = 0.13;
const BLINK_INTERVAL_MIN: f32 = 2.25;
const BLINK_INTERVAL_MAX: f32 = 12.0;
const BLINK_INTERVAL_SCALE: f32 = 4.2;
const BLINK_INTERVAL_SHAPE: f32 = 1.7;
const DOUBLE_BLINK_PROBABILITY: f32 = 0.12;
const DOUBLE_BLINK_GAP_MIN: f32 = 0.12;
const DOUBLE_BLINK_GAP_MAX: f32 = 0.22;
const YAWN_FIRST_CHECK: f32 = 3.0 * 60.0;
const YAWN_CHECK_INTERVAL: f32 = 10.0;
const YAWN_OPEN: f32 = 0.9;
const YAWN_HOLD: f32 = 1.3;
const YAWN_CLOSE: f32 = 1.0;
const SLEEP_START: f32 = 5.0 * 60.0;
const SLEEP_CLOSE: f32 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlinkPhase {
    Waiting { until: f32, is_follow_up: bool },
    Blinking { started_at: f32, is_follow_up: bool },
}

/// Produces irregular, smooth blinks without depending on a random-number crate.
pub struct Blinker {
    phase: BlinkPhase,
    random_state: u64,
}

impl Blinker {
    pub fn new(seed: u64) -> Self {
        let mut blinker = Self {
            phase: BlinkPhase::Waiting {
                until: 0.0,
                is_follow_up: false,
            },
            random_state: seed.max(1),
        };
        let delay = blinker.next_blink_interval();
        blinker.phase = BlinkPhase::Waiting {
            until: delay,
            is_follow_up: false,
        };
        blinker
    }

    /// Returns eyelid closure from `0.0` (open) through `1.0` (closed).
    pub fn closure(&mut self, elapsed: Duration) -> f32 {
        let now = elapsed.as_secs_f32();

        if let BlinkPhase::Waiting {
            until,
            is_follow_up,
        } = self.phase
        {
            if now < until {
                return 0.0;
            }
            self.phase = BlinkPhase::Blinking {
                started_at: now,
                is_follow_up,
            };
        }

        let BlinkPhase::Blinking {
            started_at,
            is_follow_up,
        } = self.phase
        else {
            return 0.0;
        };
        let blink_time = now - started_at;
        let total = BLINK_CLOSE + BLINK_HOLD + BLINK_OPEN;

        if blink_time >= total {
            if !is_follow_up && next_random_unit(&mut self.random_state) < DOUBLE_BLINK_PROBABILITY
            {
                let gap = self.next_double_blink_gap();
                self.phase = BlinkPhase::Waiting {
                    until: now + gap,
                    is_follow_up: true,
                };
            } else {
                let delay = self.next_blink_interval();
                self.phase = BlinkPhase::Waiting {
                    until: now + delay,
                    is_follow_up: false,
                };
            }
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

    fn next_blink_interval(&mut self) -> f32 {
        natural_blink_interval(next_random_unit(&mut self.random_state))
    }

    fn next_double_blink_gap(&mut self) -> f32 {
        let unit = next_random_unit(&mut self.random_state);
        DOUBLE_BLINK_GAP_MIN + (DOUBLE_BLINK_GAP_MAX - DOUBLE_BLINK_GAP_MIN) * unit
    }
}

fn natural_blink_interval(unit: f32) -> f32 {
    let truncated_cdf = 1.0
        - (-((BLINK_INTERVAL_MAX - BLINK_INTERVAL_MIN) / BLINK_INTERVAL_SCALE)
            .powf(BLINK_INTERVAL_SHAPE))
        .exp();
    BLINK_INTERVAL_MIN
        + BLINK_INTERVAL_SCALE
            * (-(1.0 - unit.clamp(0.0, 1.0) * truncated_cdf).ln()).powf(1.0 / BLINK_INTERVAL_SHAPE)
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
    next_yawn_check: f32,
    yawn_started_at: Option<f32>,
    random_state: u64,
}

impl IdleAnimator {
    pub fn new(seed: u64) -> Self {
        Self {
            last_cursor: None,
            last_input_marker: None,
            last_active_at: 0.0,
            next_yawn_check: YAWN_FIRST_CHECK,
            yawn_started_at: None,
            random_state: seed.max(1),
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
            self.next_yawn_check = YAWN_FIRST_CHECK;
            self.yawn_started_at = None;
            return IdleExpression::default();
        }

        let idle = (now - self.last_active_at).max(0.0);
        while self.next_yawn_check <= idle && self.next_yawn_check <= SLEEP_START {
            let probability = yawn_probability(self.next_yawn_check);
            if next_random_unit(&mut self.random_state) < probability {
                self.yawn_started_at = Some(self.last_active_at + self.next_yawn_check);
            }
            self.next_yawn_check += YAWN_CHECK_INTERVAL;
        }

        let yawn_time = self
            .yawn_started_at
            .map_or(-1.0, |started_at| now - started_at);
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

fn yawn_probability(idle_seconds: f32) -> f32 {
    let x = idle_seconds / YAWN_CHECK_INTERVAL;
    ((x * x - 324.0) / 576.0).clamp(0.0, 1.0)
}

// xorshift64*: sufficient for varying animation timing and yawn trials.
fn next_random_unit(random_state: &mut u64) -> f32 {
    let mut value = *random_state;
    value ^= value >> 12;
    value ^= value << 25;
    value ^= value >> 27;
    *random_state = value;
    let sample = value.wrapping_mul(0x2545_f491_4f6c_dd1d);
    (sample >> 40) as f32 / (1_u32 << 24) as f32
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
        blinker.phase = BlinkPhase::Waiting {
            until: 0.0,
            is_follow_up: false,
        };

        assert_eq!(blinker.closure(Duration::ZERO), 0.0);
        assert!(blinker.closure(Duration::from_millis(60)) > 0.8);
        assert_eq!(blinker.closure(Duration::from_millis(90)), 1.0);
        assert!(blinker.closure(Duration::from_millis(200)) < 0.7);
        assert_eq!(blinker.closure(Duration::from_millis(300)), 0.0);
    }

    #[test]
    fn blink_interval_stays_within_natural_range() {
        assert_eq!(natural_blink_interval(0.0), BLINK_INTERVAL_MIN);
        assert!(natural_blink_interval(0.5) > BLINK_INTERVAL_MIN);
        assert!(natural_blink_interval(0.5) < BLINK_INTERVAL_MAX);
        assert!((natural_blink_interval(1.0) - BLINK_INTERVAL_MAX).abs() < 0.000_01);
    }

    #[test]
    fn follow_up_blink_never_schedules_a_third_blink() {
        let mut blinker = Blinker::new(1);
        blinker.phase = BlinkPhase::Blinking {
            started_at: 0.0,
            is_follow_up: true,
        };

        assert_eq!(blinker.closure(Duration::from_secs_f32(0.3)), 0.0);
        assert!(matches!(
            blinker.phase,
            BlinkPhase::Waiting {
                is_follow_up: false,
                ..
            }
        ));
    }

    #[test]
    fn idle_pointer_yawns_then_sleeps() {
        let cursor = eframe::egui::pos2(100.0, 100.0);
        let mut idle = IdleAnimator::new(1);

        assert_eq!(
            idle.expression(Duration::ZERO, cursor, None),
            IdleExpression::default()
        );
        assert_eq!(
            idle.expression(Duration::from_secs_f32(180.8), cursor, None),
            IdleExpression::default()
        );
        assert!(
            idle.expression(Duration::from_secs_f32(300.8), cursor, None)
                .yawn
                > 0.9
        );
        assert!(
            idle.expression(Duration::from_secs_f32(301.2), cursor, None)
                .sleep
                > 0.99
        );
    }

    #[test]
    fn pointer_movement_wakes_immediately() {
        let mut idle = IdleAnimator::new(1);
        let cursor = eframe::egui::pos2(100.0, 100.0);
        idle.expression(Duration::ZERO, cursor, None);
        assert!(
            idle.expression(Duration::from_secs_f32(301.2), cursor, None)
                .sleep
                > 0.99
        );

        assert_eq!(
            idle.expression(
                Duration::from_secs_f32(301.3),
                cursor + eframe::egui::vec2(2.0, 0.0),
                None,
            ),
            IdleExpression::default()
        );
    }

    #[test]
    fn keyboard_input_prevents_idle_expression_while_mouse_is_still() {
        let mut idle = IdleAnimator::new(1);
        let cursor = eframe::egui::pos2(100.0, 100.0);
        idle.expression(Duration::ZERO, cursor, Some(100));
        assert!(
            idle.expression(Duration::from_secs_f32(301.2), cursor, Some(100))
                .sleep
                > 0.99
        );

        assert_eq!(
            idle.expression(Duration::from_secs_f32(301.3), cursor, Some(101)),
            IdleExpression::default()
        );
        assert_eq!(
            idle.expression(Duration::from_secs_f32(481.2), cursor, Some(101)),
            IdleExpression::default()
        );
        assert!(
            idle.expression(Duration::from_secs_f32(602.1), cursor, Some(101))
                .yawn
                > 0.9
        );
    }

    #[test]
    fn yawn_probability_follows_idle_time_formula() {
        assert_eq!(yawn_probability(170.0), 0.0);
        assert_eq!(yawn_probability(180.0), 0.0);
        assert!((yawn_probability(240.0) - 0.4375).abs() < f32::EPSILON);
        assert_eq!(yawn_probability(300.0), 1.0);
        assert_eq!(yawn_probability(310.0), 1.0);
    }
}
