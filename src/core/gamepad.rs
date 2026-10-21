
use gilrs::{Axis, Button, Gilrs};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GamepadState {
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub button_a: bool,
    pub button_b: bool,
    pub button_x: bool,
    pub button_y: bool,
    pub button_start: bool,
    pub button_back: bool,
    pub stick_x: f32,
    pub stick_y: f32,
}

pub struct GamepadPoller {
    gilrs: Option<Gilrs>,
}

impl GamepadPoller {
    pub fn new() -> GamepadPoller {
        GamepadPoller { gilrs: Gilrs::new().ok() }
    }

    pub fn is_available(&self) -> bool {
        self.gilrs.is_some()
    }

    // Fork: upstream read only the first pad. On a Deck that can be a device Steam isn't feeding
    // (the raw controller next to Steam Input's virtual pad), so every connected pad counts.
    pub fn poll(&mut self) -> Option<GamepadState> {
        let gilrs = self.gilrs.as_mut()?;
        while gilrs.next_event().is_some() {}
        merge_states(gilrs.gamepads().map(|(_, gamepad)| GamepadState {
            dpad_up: gamepad.is_pressed(Button::DPadUp),
            dpad_down: gamepad.is_pressed(Button::DPadDown),
            dpad_left: gamepad.is_pressed(Button::DPadLeft),
            dpad_right: gamepad.is_pressed(Button::DPadRight),
            button_a: gamepad.is_pressed(Button::South),
            button_b: gamepad.is_pressed(Button::East),
            button_x: gamepad.is_pressed(Button::West),
            button_y: gamepad.is_pressed(Button::North),
            button_start: gamepad.is_pressed(Button::Start),
            button_back: gamepad.is_pressed(Button::Select),
            stick_x: gamepad.value(Axis::LeftStickX),
            stick_y: gamepad.value(Axis::LeftStickY),
        }))
    }
}

/// A button is down if it is down on any pad; each stick axis takes the value furthest from centre
/// (the earlier pad on a tie; a non-finite reading never wins). `None` when no pad is connected.
fn merge_states(states: impl Iterator<Item = GamepadState>) -> Option<GamepadState> {
    let further = |a: f32, b: f32| if b.is_finite() && (!a.is_finite() || b.abs() > a.abs()) { b } else { a };
    states.reduce(|a, b| GamepadState {
        dpad_up: a.dpad_up || b.dpad_up,
        dpad_down: a.dpad_down || b.dpad_down,
        dpad_left: a.dpad_left || b.dpad_left,
        dpad_right: a.dpad_right || b.dpad_right,
        button_a: a.button_a || b.button_a,
        button_b: a.button_b || b.button_b,
        button_x: a.button_x || b.button_x,
        button_y: a.button_y || b.button_y,
        button_start: a.button_start || b.button_start,
        button_back: a.button_back || b.button_back,
        stick_x: further(a.stick_x, b.stick_x),
        stick_y: further(a.stick_y, b.stick_y),
    })
}

impl Default for GamepadPoller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pads_gives_no_state() {
        assert_eq!(merge_states(std::iter::empty()), None);
    }

    #[test]
    fn one_pad_passes_through_unchanged() {
        let pad = GamepadState { dpad_left: true, button_y: true, stick_x: -0.4, stick_y: 0.9, ..Default::default() };
        assert_eq!(merge_states([pad].into_iter()), Some(pad));
    }

    #[test]
    fn a_press_on_any_pad_counts() {
        // An idle first pad (the one upstream read) must not hide presses on the second.
        let idle = GamepadState::default();
        let active = GamepadState { dpad_down: true, button_a: true, button_start: true, ..Default::default() };
        let third = GamepadState { button_b: true, ..Default::default() };
        let merged = merge_states([idle, active, third].into_iter()).unwrap();
        assert!(merged.dpad_down && merged.button_a && merged.button_start && merged.button_b);
        assert!(!merged.dpad_up && !merged.button_x && !merged.button_y && !merged.button_back);
    }

    #[test]
    fn sticks_take_the_strongest_deflection_per_axis() {
        let a = GamepadState { stick_x: 0.2, stick_y: -0.95, ..Default::default() };
        let b = GamepadState { stick_x: -0.7, stick_y: 0.1, ..Default::default() };
        let merged = merge_states([a, b].into_iter()).unwrap();
        assert_eq!((merged.stick_x, merged.stick_y), (-0.7, -0.95));
    }

    #[test]
    fn stick_ties_and_bad_readings() {
        let x = |v: f32| GamepadState { stick_x: v, ..Default::default() };
        assert_eq!(merge_states([x(0.5), x(-0.5)].into_iter()).unwrap().stick_x, 0.5);
        assert_eq!(merge_states([x(f32::NAN), x(0.3)].into_iter()).unwrap().stick_x, 0.3);
        assert_eq!(merge_states([x(0.3), x(f32::NAN)].into_iter()).unwrap().stick_x, 0.3);
        assert_eq!(merge_states([x(0.1), x(f32::INFINITY)].into_iter()).unwrap().stick_x, 0.1);
    }
}
