//! Uhr: Wecker und Countdown. Ein kleiner Thread prüft jede halbe Sekunde, ob etwas
//! fällig ist – so klingelt es auch, wenn das Fenster im Tray liegt.

use std::{
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Alarm {
    pub id: u64,
    pub hour: u8,
    pub minute: u8,
    pub label: String,
    pub enabled: bool,
    /// Täglich wiederholen; sonst schaltet sich der Wecker nach dem Klingeln ab.
    pub repeat: bool,
}

/// Ein Countdown; `end` ist gesetzt, solange er läuft (Sekunden seit 1970).
#[derive(Debug, Clone, PartialEq)]
pub struct Countdown {
    pub id: u64,
    pub label: String,
    pub secs: u32,
    pub end: Option<u64>,
}

/// Zeitpunkt, wie ihn der Prüf-Thread sieht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    /// Fortlaufende Tagesnummer (nur zum Unterscheiden von Tagen).
    pub day: u32,
    pub hour: u8,
    pub minute: u8,
}

#[derive(Default)]
pub struct State {
    pub alarms: Vec<Alarm>,
    pub countdowns: Vec<Countdown>,
    /// Text dessen, was gerade klingelt.
    pub ringing: Option<String>,
    /// Der Thread hat einen einmaligen Wecker abgeschaltet – die GUI soll speichern.
    pub dirty: bool,
    /// Zuletzt ausgelöste Minute je Wecker, damit keiner doppelt klingelt.
    fired: Vec<(u64, Moment)>,
}

impl State {
    /// Prüft Wecker und Countdown; liefert `true`, wenn etwas neu klingelt.
    pub fn check(&mut self, moment: Moment, unix: u64) -> bool {
        let mut rang = false;
        for alarm in &mut self.alarms {
            if !alarm.enabled || alarm.hour != moment.hour || alarm.minute != moment.minute {
                continue;
            }
            if self.fired.contains(&(alarm.id, moment)) {
                continue;
            }
            self.fired.retain(|(id, _)| *id != alarm.id);
            self.fired.push((alarm.id, moment));
            let label = if alarm.label.trim().is_empty() {
                format!("Wecker {:02}:{:02}", alarm.hour, alarm.minute)
            } else {
                alarm.label.clone()
            };
            self.ringing = Some(label);
            if !alarm.repeat {
                alarm.enabled = false;
                self.dirty = true;
            }
            rang = true;
        }
        for countdown in &mut self.countdowns {
            if countdown.end.is_some_and(|end| unix >= end) {
                countdown.end = None;
                self.ringing = Some(if countdown.label.trim().is_empty() {
                    "Countdown abgelaufen".to_owned()
                } else {
                    format!("{}: Countdown abgelaufen", countdown.label.trim())
                });
                rang = true;
            }
        }
        rang
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Griff der GUI auf den Uhr-Thread.
pub struct ClockHandle {
    state: Arc<Mutex<State>>,
}

impl ClockHandle {
    /// `now` liefert die Ortszeit, `beep` gibt einen Signalton aus und `wake` weckt die GUI.
    pub fn spawn(
        alarms: Vec<Alarm>,
        countdowns: Vec<Countdown>,
        now: fn() -> Moment,
        beep: fn(),
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let state = Arc::new(Mutex::new(State {
            alarms,
            countdowns,
            ..State::default()
        }));
        let worker = Arc::clone(&state);
        let _ = thread::Builder::new().name("clock".into()).spawn(move || {
            let mut ticks = 0u32;
            loop {
                thread::sleep(Duration::from_millis(500));
                ticks = ticks.wrapping_add(1);
                let (rang, ringing) = {
                    let mut state = lock(&worker);
                    let rang = state.check(now(), unix_now());
                    (rang, state.ringing.is_some())
                };
                if rang {
                    wake();
                }
                // Solange es klingelt: alle anderthalb Sekunden ein Ton.
                if ringing && ticks % 3 == 0 {
                    beep();
                }
            }
        });
        Self { state }
    }

    pub fn read<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&lock(&self.state))
    }

    pub fn update<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut lock(&self.state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm(id: u64, hour: u8, minute: u8, repeat: bool) -> Alarm {
        Alarm {
            id,
            hour,
            minute,
            label: String::new(),
            enabled: true,
            repeat,
        }
    }

    #[test]
    fn rings_once_per_minute_and_disables_single_alarms() {
        let mut state = State {
            alarms: vec![alarm(1, 7, 30, false), alarm(2, 7, 30, true)],
            ..State::default()
        };
        let at = |day, hour, minute| Moment { day, hour, minute };
        assert!(!state.check(at(1, 7, 29), 0));
        assert!(state.check(at(1, 7, 30), 0));
        assert_eq!(state.ringing.as_deref(), Some("Wecker 07:30"));
        assert!(state.dirty);
        state.ringing = None;
        // Dieselbe Minute klingelt nicht noch einmal.
        assert!(!state.check(at(1, 7, 30), 0));
        // Am nächsten Tag nur noch der wiederholende Wecker.
        assert!(state.check(at(2, 7, 30), 0));
        assert!(!state.alarms[0].enabled && state.alarms[1].enabled);
    }

    #[test]
    fn countdown_fires_at_its_end() {
        let mut state = State {
            countdowns: vec![
                Countdown {
                    id: 1,
                    label: String::new(),
                    secs: 100,
                    end: Some(100),
                },
                Countdown {
                    id: 2,
                    label: "Tee".to_owned(),
                    secs: 200,
                    end: Some(200),
                },
            ],
            ..State::default()
        };
        let moment = Moment {
            day: 0,
            hour: 0,
            minute: 0,
        };
        assert!(!state.check(moment, 99));
        assert!(state.check(moment, 100));
        assert_eq!(state.countdowns[0].end, None);
        assert_eq!(state.countdowns[1].end, Some(200));
        assert_eq!(state.ringing.as_deref(), Some("Countdown abgelaufen"));
        assert!(state.check(moment, 200));
        assert_eq!(state.ringing.as_deref(), Some("Tee: Countdown abgelaufen"));
    }
}
