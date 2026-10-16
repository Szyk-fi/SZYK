//! Keyboard/pad ownership is independent of the app whose controls are visible.
use crate::app::{App, Input};

pub type Apps = [(String, Box<dyn App>)];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    #[default]
    FollowScreen,
    Off,
    Instrument(usize),
}

#[derive(Default)]
pub struct Routing {
    selection: Target,
    current: Option<usize>,
    blocked_pads: [bool; 16],
    blocked_keys: crate::app::MidiKeys,
}

impl Routing {
    pub fn pinned(&self) -> Option<usize> {
        if let Target::Instrument(i) = self.selection {
            Some(i)
        } else {
            None
        }
    }
    pub fn selection(&self) -> Target {
        self.selection
    }
    pub fn target(&self, active: Option<usize>) -> Option<usize> {
        match self.selection {
            Target::FollowScreen => active,
            Target::Off => None,
            Target::Instrument(i) => Some(i),
        }
    }
    pub fn options(apps: &Apps) -> Vec<Target> {
        [Target::FollowScreen, Target::Off]
            .into_iter()
            .chain(
                apps.iter()
                    .enumerate()
                    .filter(|(_, (_, a))| a.supports_pad_lock() || a.play_surface())
                    .map(|(i, _)| Target::Instrument(i)),
            )
            .collect()
    }
    pub fn option_names(apps: &Apps) -> Vec<String> {
        Self::options(apps)
            .into_iter()
            .map(|i| match i {
                Target::FollowScreen => "Follow screen".into(),
                Target::Off => "MIDI disarmed".into(),
                Target::Instrument(i) => apps[i].0.clone(),
            })
            .collect()
    }
    pub fn selected(&self, apps: &Apps) -> usize {
        Self::options(apps)
            .iter()
            .position(|i| *i == self.selection)
            .unwrap_or(0)
    }
    pub fn select(&mut self, option: usize, apps: &Apps) {
        if let Some(target) = Self::options(apps).get(option) {
            self.set_selection(*target, apps);
        }
    }
    pub fn set_target(&mut self, target: Option<usize>, apps: &Apps) {
        self.set_selection(
            target.map_or(Target::FollowScreen, Target::Instrument),
            apps,
        );
    }
    pub fn set_selection(&mut self, target: Target, apps: &Apps) {
        if Self::options(apps).contains(&target) {
            self.selection = target;
        }
    }
    pub fn toggle(&mut self, active: Option<usize>, apps: &Apps) {
        if self.pinned().is_some() {
            self.selection = Target::Off;
        } else if let Some(i) =
            active.filter(|i| Self::options(apps).contains(&Target::Instrument(*i)))
        {
            self.selection = Target::Instrument(i);
        }
    }
    pub fn captures_keyboard(&self, active: Option<usize>, apps: &Apps) -> bool {
        self.target(active)
            .and_then(|i| apps.get(i))
            .is_some_and(|(_, a)| a.supports_pad_lock() || a.play_surface())
    }

    /// Called once per UI frame, before background work and the foreground tick.
    /// Only the performance owner gets keys, pressure and pads; view controls
    /// continue to reach the foreground app. A background owner gets one tick.
    pub fn route(&mut self, active: Option<usize>, apps: &mut Apps, input: Input) -> Input {
        let target = self.target(active).filter(|i| *i < apps.len());
        if target != self.current {
            if let Some(old) = self.current.filter(|i| *i < apps.len()) {
                apps[old].1.tick(&Input::default());
                apps[old].1.set_performance_active(false);
                // Explicit reassignment releases the old owner and waits for
                // held physical controls to be released before the new owner.
                for i in 0..16 {
                    self.blocked_pads[i] |= input.grid[i];
                }
                for i in 0..128 {
                    self.blocked_keys.0[i] |= input.midi_keys.0[i];
                }
            }
            if let Some(i) = target {
                apps[i].1.set_performance_active(true);
            }
            self.current = target;
        }
        let mut performance = performance_input(input);
        if target.is_none() {
            for i in 0..16 {
                self.blocked_pads[i] |= input.grid[i];
            }
            for i in 0..128 {
                self.blocked_keys.0[i] |= input.midi_keys.0[i];
            }
        }
        for i in 0..16 {
            self.blocked_pads[i] &= input.grid[i];
            if self.blocked_pads[i] {
                performance.grid[i] = false;
                performance.pad_pressure[i] = 0.;
            }
        }
        for i in 0..128 {
            if input.midi_keys.0[i] == 0 {
                self.blocked_keys.0[i] = 0;
            }
            if self.blocked_keys.0[i] > 0 {
                performance.midi_keys.0[i] = 0;
            }
        }
        let mut screen = input;
        copy_performance(&mut screen, &Input::default());
        if let Some(i) = target {
            if Some(i) == active {
                copy_performance(&mut screen, &performance);
            } else {
                apps[i].1.tick(&performance);
            }
        }
        screen
    }
}

pub fn performance_input(input: Input) -> Input {
    let mut performance = Input::default();
    copy_performance(&mut performance, &input);
    performance
}

fn copy_performance(to: &mut Input, from: &Input) {
    to.grid = from.grid;
    to.pad_pressure = from.pad_pressure;
    to.midi_keys = from.midi_keys;
    to.pitch_bend = from.pitch_bend;
    to.mod_wheel = from.mod_wheel;
    to.aftertouch = from.aftertouch;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::FrameBuffer;
    use std::{cell::RefCell, rc::Rc};
    struct Probe(Rc<RefCell<Vec<Input>>>);
    impl App for Probe {
        fn tick(&mut self, i: &Input) {
            self.0.borrow_mut().push(*i);
        }
        fn draw(&mut self, _: &mut FrameBuffer) {}
        fn supports_pad_lock(&self) -> bool {
            true
        }
    }
    fn apps() -> (Vec<(String, Box<dyn App>)>, Vec<Rc<RefCell<Vec<Input>>>>) {
        let frames: Vec<Rc<RefCell<Vec<Input>>>> = vec![Rc::default(), Rc::default()];
        let apps = ["Bebot", "Beads"]
            .iter()
            .enumerate()
            .map(|(i, n)| {
                (
                    (*n).into(),
                    Box::new(Probe(frames[i].clone())) as Box<dyn App>,
                )
            })
            .collect();
        (apps, frames)
    }
    fn held() -> Input {
        let mut i = Input {
            knob2: 7,
            navigation_steps: 3,
            pitch_bend: 0.3,
            mod_wheel: 0.4,
            aftertouch: 0.7,
            ..Default::default()
        };
        i.grid[2] = true;
        i.pad_pressure[2] = 0.8;
        i.midi_keys.0[60] = 99;
        i
    }
    #[test]
    fn pinned_keys_and_pads_keep_playing_while_controls_follow_another_app_and_home() {
        let (mut apps, frames) = apps();
        let mut routing = Routing::default();
        let input = routing.route(Some(0), &mut apps, held());
        apps[0].1.tick(&input);
        routing.toggle(Some(0), &apps);
        for active in [Some(1), None, Some(1)] {
            let screen = routing.route(active, &mut apps, held());
            assert_eq!(screen.midi_keys.0[60], 0);
            assert!(!screen.grid[2]);
            assert_eq!(screen.knob2, 7);
            assert_eq!(screen.navigation_steps, 3);
            let played = *frames[0].borrow().last().unwrap();
            assert_eq!(played.midi_keys.0[60], 99);
            assert!(played.grid[2]);
            assert_eq!(played.knob2, 0);
            assert_eq!(played.navigation_steps, 0);
            assert_eq!(played.pitch_bend, 0.3);
            assert_eq!(played.pad_pressure[2], 0.8);
            if active == Some(1) {
                apps[1].1.tick(&screen);
            }
        }
        assert_eq!(
            frames[0].borrow().len(),
            4,
            "one performance tick per frame, no transition release"
        );
        routing.route(Some(1), &mut apps, Input::default());
        assert_eq!(frames[0].borrow().last().unwrap().midi_keys.0[60], 0);
    }
    #[test]
    fn reassignment_releases_old_notes_and_waits_for_physical_release_before_new_owner() {
        let (mut apps, frames) = apps();
        let mut routing = Routing::default();
        let i = routing.route(Some(0), &mut apps, held());
        apps[0].1.tick(&i);
        routing.set_target(Some(1), &apps);
        let screen = routing.route(Some(0), &mut apps, held());
        apps[0].1.tick(&screen);
        assert_eq!(frames[0].borrow().last().unwrap().midi_keys.0[60], 0);
        assert_eq!(frames[1].borrow().last().unwrap().midi_keys.0[60], 0);
        assert!(!frames[1].borrow().last().unwrap().grid[2]);
        routing.route(Some(0), &mut apps, Input::default());
        routing.route(Some(0), &mut apps, held());
        assert_eq!(frames[1].borrow().last().unwrap().midi_keys.0[60], 99);
        assert!(frames[1].borrow().last().unwrap().grid[2]);
        assert_eq!(routing.target(Some(0)), Some(1));
        routing.select(0, &apps);
        assert_eq!(routing.target(Some(0)), Some(0));
    }
    #[test]
    fn external_midi_keeps_note_numbers_and_releases_while_a_non_play_screen_is_visible() {
        let c = crate::controller::ControllerState::new();
        c.set_play_surface(true);
        c.set_keyboard_target_active(true);
        assert!(c.play_surface_midi(0x90, 84, 110));
        c.set_play_surface(false);
        assert_eq!(c.play_surface_input(Input::default()).midi_keys.0[84], 110);
        assert!(c.play_surface_midi(0x80, 84, 0));
        assert_eq!(c.play_surface_input(Input::default()).midi_keys.0[84], 0);
        assert!(c.play_surface_midi(0x90, 85, 90));
        c.set_keyboard_target_active(false);
        assert_eq!(c.play_surface_input(Input::default()).midi_keys.0[85], 0);
        c.grid[0].store(true, std::sync::atomic::Ordering::Relaxed);
        c.set_keyboard_target_active(true);
        assert!(c.play_surface_midi(0x80, 60, 0));
        assert!(
            !c.pad_down(0),
            "a note released after arming must also clear its legacy pad state"
        );
    }
    #[test]
    fn disarm_stops_performance_and_follow_screen_is_an_explicit_separate_choice() {
        let (mut apps, frames) = apps();
        let mut routing = Routing::default();
        routing.toggle(Some(0), &apps);
        let frame = routing.route(Some(0), &mut apps, held());
        apps[0].1.tick(&frame);
        routing.toggle(Some(1), &apps);
        assert_eq!(routing.selection(), Target::Off);
        let screen = routing.route(Some(1), &mut apps, held());
        assert_eq!(screen.midi_keys.0[60], 0);
        assert!(!screen.grid[2]);
        assert_eq!(routing.target(Some(1)), None);
        assert_eq!(frames[0].borrow().last().unwrap().midi_keys.0[60], 0);
        routing.select(0, &apps);
        assert_eq!(routing.selection(), Target::FollowScreen);
        assert_eq!(routing.target(Some(1)), Some(1));
        let screen = routing.route(Some(1), &mut apps, held());
        assert_eq!(
            screen.midi_keys.0[60], 0,
            "held keys cannot jump onto the newly selected instrument"
        );
    }
    #[test]
    fn every_installed_midi_instrument_is_an_arm_target_without_opening_it() {
        let mut context = crate::app::AppContext::new();
        context
            .provide(std::sync::Arc::new(crate::audio_bus::AudioBus::new()))
            .provide(std::sync::Arc::new(crate::modbus::ModBus::new()))
            .provide(std::sync::Arc::new(crate::mixer_bus::MixerBus::new()))
            .provide(std::sync::Arc::new(crate::note_bus::NoteBus::new()));
        let registry = crate::registry::Registry::new(context);
        let manifests = crate::manifest::discover(std::path::Path::new("apps"));
        let apps = registry.build(&manifests);
        let options = Routing::options(&apps);
        let mut count = 0;
        for manifest in manifests
            .iter()
            .filter(|m| m.notes_in && registry.implements(m))
        {
            let index = apps.iter().position(|(n, _)| n == &manifest.name).unwrap();
            assert!(
                options.contains(&Target::Instrument(index)),
                "{} must be armable",
                manifest.name
            );
            count += 1;
        }
        assert!(count > 20, "exercise the full installed instrument catalog");
    }
}
