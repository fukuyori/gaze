use std::time::Duration;

use eframe::egui::{Pos2, Vec2};

const BLINK_CLOSE: f32 = 0.075;
const BLINK_HOLD: f32 = 0.045;
const BLINK_OPEN: f32 = 0.13;
// The floor sits at three seconds: anything shorter reads as a fidget rather
// than a blink, and two flickers in quick succession are what draws a working
// eye away from what it was doing.
const BLINK_INTERVAL_MIN: f32 = 3.0;
const BLINK_INTERVAL_MAX: f32 = 13.0;
const BLINK_INTERVAL_SCALE: f32 = 4.4;
const BLINK_INTERVAL_SHAPE: f32 = 1.7;
// Rare enough to read as a gesture rather than as noise: about one every two
// minutes instead of one a minute.
const DOUBLE_BLINK_PROBABILITY: f32 = 0.06;
const DOUBLE_BLINK_GAP_MIN: f32 = 0.12;
const DOUBLE_BLINK_GAP_MAX: f32 = 0.22;
// A long, deliberate close, four times the length of an ordinary blink. One
// noticeable blink now and then carries further than a faster stream of them,
// without adding to the flicker in the corner of the eye.
const SLOW_BLINK_CLOSE: f32 = 0.32;
const SLOW_BLINK_HOLD: f32 = 0.20;
const SLOW_BLINK_OPEN: f32 = 0.44;
const SLOW_BLINK_PROBABILITY: f32 = 0.05;
/// Never two slow blinks close together; they are meant to stand out.
const SLOW_BLINK_MIN_GAP: f32 = 90.0;
const YAWN_FIRST_CHECK: f32 = 3.0 * 60.0;
const YAWN_CHECK_INTERVAL: f32 = 10.0;
const YAWN_OPEN: f32 = 0.9;
const YAWN_HOLD: f32 = 1.3;
const YAWN_CLOSE: f32 = 1.0;
const SLEEP_START: f32 = 5.0 * 60.0;
const SLEEP_CLOSE: f32 = 1.2;

/// How often Gaze blinks, as offered in the settings menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlinkRate {
    #[default]
    Standard,
    Slow,
    Off,
}

impl BlinkRate {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Slow, Self::Off];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Slow => "Slow",
            Self::Off => "Off",
        }
    }

    /// The name this is written under in the settings file.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Slow => "slow",
            Self::Off => "off",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rate| rate.key() == key)
    }

    /// How much longer the wait between blinks is than at the standard rate.
    /// Doubling it lands near the rate a person reading holds.
    const fn interval_factor(self) -> f32 {
        match self {
            Self::Standard | Self::Off => 1.0,
            Self::Slow => 2.0,
        }
    }
}

/// How long a blink spends closing, staying shut, and opening again.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BlinkShape {
    close: f32,
    hold: f32,
    open: f32,
}

impl BlinkShape {
    const QUICK: Self = Self {
        close: BLINK_CLOSE,
        hold: BLINK_HOLD,
        open: BLINK_OPEN,
    };
    const SLOW: Self = Self {
        close: SLOW_BLINK_CLOSE,
        hold: SLOW_BLINK_HOLD,
        open: SLOW_BLINK_OPEN,
    };

    const fn of(slow: bool) -> Self {
        if slow { Self::SLOW } else { Self::QUICK }
    }

    fn total(self) -> f32 {
        self.close + self.hold + self.open
    }

    fn closure(self, blink_time: f32) -> f32 {
        if blink_time < self.close {
            smoothstep(blink_time / self.close)
        } else if blink_time < self.close + self.hold {
            1.0
        } else {
            1.0 - smoothstep((blink_time - self.close - self.hold) / self.open)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlinkPhase {
    Waiting {
        until: f32,
        is_follow_up: bool,
        slow: bool,
    },
    Blinking {
        started_at: f32,
        is_follow_up: bool,
        slow: bool,
    },
}

/// Produces irregular, smooth blinks without depending on a random-number crate.
pub struct Blinker {
    phase: BlinkPhase,
    last_slow_at: f32,
    random_state: u64,
}

impl Blinker {
    pub fn new(seed: u64) -> Self {
        let mut blinker = Self {
            phase: BlinkPhase::Waiting {
                until: 0.0,
                is_follow_up: false,
                slow: false,
            },
            last_slow_at: 0.0,
            random_state: seed.max(1),
        };
        let delay = blinker.next_blink_interval();
        blinker.phase = BlinkPhase::Waiting {
            until: delay,
            is_follow_up: false,
            slow: false,
        };
        blinker
    }

    /// Returns eyelid closure from `0.0` (open) through `1.0` (closed).
    pub fn closure(&mut self, elapsed: Duration, rate: BlinkRate) -> f32 {
        let now = elapsed.as_secs_f32();

        if rate == BlinkRate::Off {
            // Hold the schedule just out of reach rather than letting it fall
            // behind, so turning blinking back on does not fire one at once.
            let overdue = match self.phase {
                BlinkPhase::Waiting { until, .. } => now >= until,
                BlinkPhase::Blinking { .. } => true,
            };
            if overdue {
                self.phase = BlinkPhase::Waiting {
                    until: now + BLINK_INTERVAL_MIN,
                    is_follow_up: false,
                    slow: false,
                };
            }
            return 0.0;
        }

        if let BlinkPhase::Waiting {
            until,
            is_follow_up,
            slow,
        } = self.phase
        {
            if now < until {
                return 0.0;
            }
            self.phase = BlinkPhase::Blinking {
                started_at: now,
                is_follow_up,
                slow,
            };
        }

        let BlinkPhase::Blinking {
            started_at,
            is_follow_up,
            slow,
        } = self.phase
        else {
            return 0.0;
        };
        let shape = BlinkShape::of(slow);
        let blink_time = now - started_at;

        if blink_time >= shape.total() {
            self.schedule_next(now, is_follow_up, slow, rate);
            return 0.0;
        }

        shape.closure(blink_time)
    }

    fn schedule_next(&mut self, now: f32, is_follow_up: bool, was_slow: bool, rate: BlinkRate) {
        // A slow blink is a gesture in its own right, so it is never the first
        // half of a double one.
        if !is_follow_up
            && !was_slow
            && next_random_unit(&mut self.random_state) < DOUBLE_BLINK_PROBABILITY
        {
            let gap = self.next_double_blink_gap();
            self.phase = BlinkPhase::Waiting {
                until: now + gap,
                is_follow_up: true,
                slow: false,
            };
            return;
        }

        if was_slow {
            self.last_slow_at = now;
        }
        let until = now + self.next_blink_interval() * rate.interval_factor();
        let slow = self.roll_slow_blink(until);
        self.phase = BlinkPhase::Waiting {
            until,
            is_follow_up: false,
            slow,
        };
    }

    fn roll_slow_blink(&mut self, at: f32) -> bool {
        at - self.last_slow_at >= SLOW_BLINK_MIN_GAP
            && next_random_unit(&mut self.random_state) < SLOW_BLINK_PROBABILITY
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
///
/// `far` is half the monitor diagonal because that is roughly the farthest the
/// cursor can get from a window near the middle of the screen; a longer range
/// would mean the eyes never reach a full squint on a single display.
pub fn distance_squint(distance: f32, monitor_diagonal: f32) -> f32 {
    let near = (monitor_diagonal * 0.16).max(240.0);
    let far = (monitor_diagonal * 0.5).max(near + 1.0);
    smoothstep(((distance - near) / (far - near)).clamp(0.0, 1.0))
}

const PURSUIT_RATE: f32 = 13.0;
const SACCADE_RATE: f32 = 46.0;
const SACCADE_DISTANCE: f32 = 260.0;

/// Smooths the point the eyes aim at so they follow the cursor like eyes do.
///
/// Real eyes neither teleport nor glide at one speed: a nearby target is
/// tracked with a lazy smooth pursuit, while a distant one is caught with a
/// fast saccade. Feeding the aim point through a critically damped spring whose
/// stiffness grows with the distance to cover reproduces both, and being
/// critically damped it settles without the rubbery overshoot of a plain
/// spring.
#[derive(Default)]
pub struct GazeTracker {
    position: Option<Pos2>,
    velocity: Vec2,
}

impl GazeTracker {
    pub fn follow(&mut self, target: Pos2, delta_time: f32) -> Pos2 {
        let Some(position) = self.position else {
            self.position = Some(target);
            return target;
        };

        // A stalled frame must not fling the eyes past the cursor.
        let delta_time = delta_time.clamp(0.0, 0.1);
        if delta_time <= 0.0 {
            return position;
        }

        let rate = PURSUIT_RATE
            + (SACCADE_RATE - PURSUIT_RATE)
                * smoothstep(position.distance(target) / SACCADE_DISTANCE);

        // Semi-implicit critically damped spring; stable at any frame time.
        let damping = 1.0 + 2.0 * delta_time * rate;
        let pull = delta_time * rate * rate;
        let weight = delta_time * pull;
        let inverse = 1.0 / (damping + weight);
        let next =
            (position.to_vec2() * damping + self.velocity * delta_time + target.to_vec2() * weight)
                * inverse;
        self.velocity = (self.velocity + (target - position) * pull) * inverse;
        self.position = Some(next.to_pos2());
        next.to_pos2()
    }
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

    impl Blinker {
        /// The rate every test but the ones about the setting itself uses.
        fn closure_std(&mut self, elapsed: Duration) -> f32 {
            self.closure(elapsed, BlinkRate::Standard)
        }
    }

    #[test]
    fn nearby_cursor_does_not_squint() {
        assert_eq!(distance_squint(100.0, 2_000.0), 0.0);
    }

    #[test]
    fn remote_cursor_reaches_full_squint() {
        assert_eq!(distance_squint(2_000.0, 2_000.0), 1.0);
    }

    #[test]
    fn a_screen_corner_fully_squints_a_centred_window() {
        // 1512x982 points: the corner sits half a diagonal from the middle.
        let diagonal = eframe::egui::vec2(1_512.0, 982.0).length();
        assert_eq!(distance_squint(diagonal * 0.5, diagonal), 1.0);
        let halfway = distance_squint(diagonal * 0.33, diagonal);
        assert!(halfway > 0.15 && halfway < 0.85, "{halfway}");
    }

    #[test]
    fn a_cursor_inside_the_window_keeps_the_eyes_wide_open() {
        let diagonal = eframe::egui::vec2(1_512.0, 982.0).length();
        let window_corner = eframe::egui::vec2(190.0, 110.0).length();
        assert_eq!(distance_squint(window_corner, diagonal), 0.0);
    }

    #[test]
    fn gaze_approaches_the_cursor_steadily_and_settles_on_it() {
        let mut gaze = GazeTracker::default();
        let start = eframe::egui::pos2(0.0, 0.0);
        let target = eframe::egui::pos2(400.0, 0.0);
        assert_eq!(gaze.follow(start, 1.0 / 60.0), start);

        let mut position = start;
        let mut furthest = start.x;
        for frame in 0..60 {
            let next = gaze.follow(target, 1.0 / 60.0);
            // The sweep towards the cursor never stutters or swings back.
            if frame < 10 {
                assert!(next.x > position.x, "frame {frame}: {next:?}");
            }
            furthest = furthest.max(next.x);
            position = next;
        }

        // Landing needs at most the whisper of a correction a real saccade
        // makes, never a rubbery bounce.
        assert!(furthest <= target.x * 1.02, "{furthest}");
        assert!((position.x - target.x).abs() < 1.0, "{position:?}");
    }

    #[test]
    fn gaze_lags_behind_a_jump_but_catches_a_nudge_quickly() {
        let origin = eframe::egui::pos2(0.0, 0.0);
        let frame = 1.0 / 60.0;

        let mut jumped = GazeTracker::default();
        jumped.follow(origin, frame);
        let after_jump = jumped.follow(eframe::egui::pos2(600.0, 0.0), frame);
        assert!(after_jump.x < 600.0 * 0.5, "a jump is not teleported");

        let mut nudged = GazeTracker::default();
        nudged.follow(origin, frame);
        let after_nudge = nudged.follow(eframe::egui::pos2(6.0, 0.0), frame);
        assert!(after_nudge.x > 0.0, "a small move still starts immediately");
    }

    #[test]
    fn a_stalled_frame_does_not_fling_the_gaze() {
        let mut gaze = GazeTracker::default();
        gaze.follow(eframe::egui::pos2(0.0, 0.0), 1.0 / 60.0);
        let target = eframe::egui::pos2(500.0, 0.0);
        let position = gaze.follow(target, 5.0);
        assert!(
            position.x <= target.x,
            "a stall must not fling the gaze past"
        );
        assert!(position.x > 300.0, "a long frame still catches most of it");
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
            slow: false,
        };

        assert_eq!(blinker.closure(Duration::ZERO, BlinkRate::Standard), 0.0);
        assert!(blinker.closure_std(Duration::from_millis(60)) > 0.8);
        assert_eq!(blinker.closure_std(Duration::from_millis(90)), 1.0);
        assert!(blinker.closure_std(Duration::from_millis(200)) < 0.7);
        assert_eq!(blinker.closure_std(Duration::from_millis(300)), 0.0);
    }

    #[test]
    fn a_slow_blink_lingers_where_an_ordinary_one_is_already_over() {
        let mut blinker = Blinker::new(1);
        blinker.phase = BlinkPhase::Waiting {
            until: 0.0,
            is_follow_up: false,
            slow: true,
        };

        // The blink starts at the first call past its due time.
        assert_eq!(blinker.closure(Duration::ZERO, BlinkRate::Standard), 0.0);

        // Still closing where a quick blink has come and gone.
        assert!(blinker.closure_std(Duration::from_millis(200)) < 1.0);
        assert_eq!(blinker.closure_std(Duration::from_millis(330)), 1.0);
        assert_eq!(blinker.closure_std(Duration::from_millis(500)), 1.0);
        assert!(blinker.closure_std(Duration::from_millis(800)) > 0.0);
        assert_eq!(blinker.closure_std(Duration::from_millis(970)), 0.0);

        assert!(BlinkShape::SLOW.total() > 3.5 * BlinkShape::QUICK.total());
    }

    #[test]
    fn a_slow_blink_is_never_half_of_a_double_blink() {
        let mut blinker = Blinker::new(1);
        for seed in 1..200_u64 {
            blinker.random_state = seed;
            blinker.phase = BlinkPhase::Blinking {
                started_at: 0.0,
                is_follow_up: false,
                slow: true,
            };
            blinker.closure_std(Duration::from_secs_f32(BlinkShape::SLOW.total()));
            assert!(
                matches!(
                    blinker.phase,
                    BlinkPhase::Waiting {
                        is_follow_up: false,
                        ..
                    }
                ),
                "seed {seed} chased a slow blink with a second one"
            );
        }
    }

    /// Half an hour of blinking, sampled finely enough to catch every phase.
    /// Returns when each blink started and which of them were slow.
    fn simulate_blinks(seed: u64, minutes: f32, rate: BlinkRate) -> (Vec<f32>, Vec<f32>) {
        let mut blinker = Blinker::new(seed);
        let (mut started, mut slow_at) = (Vec::new(), Vec::new());
        let mut closed = false;
        let mut time = 0.0_f32;

        while time < minutes * 60.0 {
            let closure = blinker.closure(Duration::from_secs_f32(time), rate);
            if closure > 0.0 && !closed {
                closed = true;
                started.push(time);
                if let BlinkPhase::Blinking { slow: true, .. } = blinker.phase {
                    slow_at.push(time);
                }
            } else if closure == 0.0 {
                closed = false;
            }
            time += 0.01;
        }
        (started, slow_at)
    }

    #[test]
    fn slow_blinks_stay_far_apart_and_stay_rare() {
        for seed in 1..=5_u64 {
            let (started, slow_at) = simulate_blinks(seed, 30.0, BlinkRate::Standard);

            assert!(
                !slow_at.is_empty(),
                "seed {seed}: no slow blink in half an hour"
            );
            for pair in slow_at.windows(2) {
                assert!(
                    pair[1] - pair[0] >= SLOW_BLINK_MIN_GAP,
                    "seed {seed}: slow blinks {:.1}s apart",
                    pair[1] - pair[0]
                );
            }

            let per_minute = started.len() as f32 / 30.0;
            assert!(
                (8.0..10.5).contains(&per_minute),
                "seed {seed}: {per_minute:.1} blinks a minute"
            );
        }
    }

    #[test]
    fn the_slow_setting_roughly_halves_how_often_gaze_blinks() {
        for seed in 1..=5_u64 {
            let (standard, _) = simulate_blinks(seed, 30.0, BlinkRate::Standard);
            let (slow, _) = simulate_blinks(seed, 30.0, BlinkRate::Slow);

            let per_minute = slow.len() as f32 / 30.0;
            assert!(
                (3.5..5.5).contains(&per_minute),
                "seed {seed}: {per_minute:.1} blinks a minute at the slow setting"
            );
            assert!(slow.len() * 2 < standard.len() * 3, "seed {seed}");
        }
    }

    #[test]
    fn turning_blinking_off_keeps_the_eyes_open() {
        for seed in 1..=5_u64 {
            let (blinks, _) = simulate_blinks(seed, 30.0, BlinkRate::Off);
            assert!(blinks.is_empty(), "seed {seed}: blinked with blinking off");
        }
    }

    #[test]
    fn turning_blinking_back_on_does_not_fire_one_straight_away() {
        // Without holding the schedule forward, an hour with blinking off
        // would leave a blink overdue and it would go off on the next frame.
        let mut blinker = Blinker::new(3);
        let mut time = 0.0_f32;
        while time < 3_600.0 {
            assert_eq!(
                blinker.closure(Duration::from_secs_f32(time), BlinkRate::Off),
                0.0
            );
            time += 0.5;
        }

        assert_eq!(
            blinker.closure(Duration::from_secs_f32(time), BlinkRate::Standard),
            0.0,
            "blinked the instant blinking came back on"
        );
    }

    #[test]
    fn every_blink_rate_survives_the_settings_file() {
        for rate in BlinkRate::ALL {
            assert_eq!(BlinkRate::from_key(rate.key()), Some(rate));
        }
        assert_eq!(BlinkRate::from_key("nonsense"), None);
    }

    #[test]
    fn quick_blinks_never_crowd_each_other_outside_a_double_blink() {
        // The point of the three-second floor: two flickers in quick
        // succession are what pulls a working eye away from its work.
        for seed in 1..=5_u64 {
            let (started, _) = simulate_blinks(seed, 30.0, BlinkRate::Standard);
            let mut doubles = 0;

            for pair in started.windows(2) {
                let gap = pair[1] - pair[0];
                if gap < 1.0 {
                    doubles += 1;
                    assert!(gap > 0.3, "seed {seed}: two blinks {gap:.2}s apart");
                } else {
                    assert!(
                        gap >= BLINK_INTERVAL_MIN,
                        "seed {seed}: blinks only {gap:.2}s apart"
                    );
                }
            }

            let per_hour = f32::from(doubles as u16) * 2.0;
            assert!(
                (10.0..50.0).contains(&per_hour),
                "seed {seed}: {per_hour:.0} double blinks an hour"
            );
        }
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
            slow: false,
        };

        assert_eq!(blinker.closure_std(Duration::from_secs_f32(0.3)), 0.0);
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
