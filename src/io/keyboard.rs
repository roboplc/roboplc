use std::{
    collections::{BTreeSet, VecDeque},
    thread,
    time::Duration,
};

use crate::{Error, Result};
use bma_ts::Monotonic;
use evdev::Device;
pub use evdev::KeyCode;
use nix::sys::epoll;
use tracing::error;

/// Key state
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeyState {
    /// Key pressed
    Pressed,
    /// Key released
    Released,
    /// Other key state
    Other(i32),
}

/// Keyboard event
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct KeyEvent {
    code: KeyCode,
    state: KeyState,
    time: Monotonic,
}

impl KeyEvent {
    /// Key code
    pub fn code(&self) -> KeyCode {
        self.code
    }
    /// Key state
    pub fn state(&self) -> KeyState {
        self.state
    }
    /// Event time (monotonic)
    pub fn time(&self) -> Monotonic {
        self.time
    }
}

impl From<i32> for KeyState {
    fn from(v: i32) -> Self {
        match v {
            0 => KeyState::Released,
            1 => KeyState::Pressed,
            v => KeyState::Other(v),
        }
    }
}

/// Creates a global key listener that listens for key events on all input devices
pub struct GlobalKeyListener {
    keys: BTreeSet<KeyCode>,
    poll: epoll::Epoll,
    devices: Vec<Device>,
    epoll_events: [epoll::EpollEvent; 2],
    events_pending: VecDeque<KeyEvent>,
}

impl GlobalKeyListener {
    /// Create a new global key listener from a list of key codes and devices in `/dev/input`
    pub fn create(keys: &[KeyCode]) -> Result<Self> {
        let keys: BTreeSet<_> = keys.iter().copied().collect();
        let dir = std::fs::read_dir("/dev/input")?;
        let poll = epoll::Epoll::new(epoll::EpollCreateFlags::EPOLL_CLOEXEC).map_err(Error::io)?;
        let event = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0);
        let mut devices = Vec::new();
        for entry in dir {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            let Ok(dev) = Device::open(&path) else {
                continue;
            };
            if let Err(e) = dev.set_nonblocking(true) {
                error!(%e, name=?dev.name(), "Failed to set device non-blocking");
                continue;
            }
            let Some(supported_keys) = dev.supported_keys() else {
                continue;
            };
            let mut need_to_listen = false;
            for key in &keys {
                if supported_keys.contains(*key) {
                    need_to_listen = true;
                    break;
                }
            }
            if need_to_listen {
                if let Err(error) = poll.add(&dev, event) {
                    error!(%error, "Failed to add device to epoll");
                }
            }
            devices.push(dev);
        }
        Ok(Self {
            keys,
            poll,
            devices,
            epoll_events: [epoll::EpollEvent::empty(); 2],
            events_pending: VecDeque::with_capacity(32),
        })
    }
}

impl Iterator for GlobalKeyListener {
    type Item = KeyEvent;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(ev) = self.events_pending.pop_front() {
            return Some(ev);
        }
        loop {
            for dev in &mut self.devices {
                if let Ok(event_list) = dev.fetch_events() {
                    for ev in event_list {
                        if let evdev::EventSummary::Key(_kev, code, pressed) = ev.destructure() {
                            if self.keys.contains(&code) {
                                let state = KeyState::from(pressed);
                                let key_event = KeyEvent {
                                    code,
                                    state,
                                    time: Monotonic::now(),
                                };
                                self.events_pending.push_back(key_event);
                            }
                        }
                    }
                }
            }
            if let Some(ev) = self.events_pending.pop_front() {
                return Some(ev);
            }
            if let Err(e) = self
                .poll
                .wait(&mut self.epoll_events, epoll::EpollTimeout::NONE)
            {
                error!(%e, "Failed to wait for events in poll");
                thread::sleep(Duration::from_millis(100));
                continue;
            }
        }
    }
}
