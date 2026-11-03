//! Comportamiento autónomo de la mascota en el escritorio: pasear, mirar el cursor y reaccionar.
//! Qué hacer lo decide `zen-character` (lógica pura); aquí solo se ejecuta sobre la ventana.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};
use zen_character::{Character, State, Wander};
use zen_core::events::{Event, EventBus};
use zen_desktop::{effective_direction, facing_towards, position_after, Facing, WalkBounds};

/// Píxeles por segundo al caminar.
const WALK_SPEED: f64 = 50.0;
const EDGE_MARGIN: i32 = 8;
const LOOK_RADIUS: f64 = 350.0;
const LOOK_DEAD_ZONE: f64 = 25.0;

/// Interruptores del usuario y datos que comparten los hilos de comportamiento.
pub struct BehaviorFlags {
    pub wander: AtomicBool,
    pub look_at_cursor: AtomicBool,
    walking: AtomicBool,
    sleeping: AtomicBool,
    chat_open: AtomicBool,
}

impl BehaviorFlags {
    pub fn new(wander: bool, look_at_cursor: bool) -> Self {
        Self {
            wander: AtomicBool::new(wander),
            look_at_cursor: AtomicBool::new(look_at_cursor),
            walking: AtomicBool::new(false),
            sleeping: AtomicBool::new(false),
            chat_open: AtomicBool::new(false),
        }
    }
}

struct WalkPlan {
    direction: i32,
    distance: u32,
}

pub fn spawn(app: AppHandle, bus: Arc<EventBus>, flags: Arc<BehaviorFlags>) {
    let (walk_tx, walk_rx) = mpsc::channel();
    // Verdadero cuando hay que abandonar el paseo en curso (la mascota ya no está en estado Walking).
    let cancel = Arc::new(AtomicBool::new(true));
    spawn_walker(app.clone(), walk_rx, cancel.clone(), bus.clone());
    spawn_cursor_watcher(app.clone(), flags.clone());
    spawn_director(app, bus, flags, walk_tx, cancel);
}

/// Alimenta al personaje con los eventos del bus, avanza sus temporizadores y deja que decida
/// por su cuenta cuándo pasear o jugar.
fn spawn_director(
    app: AppHandle,
    bus: Arc<EventBus>,
    flags: Arc<BehaviorFlags>,
    walk_tx: mpsc::Sender<WalkPlan>,
    cancel: Arc<AtomicBool>,
) {
    let events = bus.subscribe();
    std::thread::spawn(move || {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        let mut character = Character::new(Instant::now());
        let mut wander = Wander::new(Instant::now(), seed);
        loop {
            let now = Instant::now();
            let mut plan = None;
            let changed = match events.recv_timeout(Duration::from_millis(250)) {
                Ok(event) => {
                    match event {
                        Event::ChatOpened => flags.chat_open.store(true, Ordering::Relaxed),
                        Event::ChatClosed => flags.chat_open.store(false, Ordering::Relaxed),
                        _ => {}
                    }
                    let changed = character.handle(&event, now);
                    if let (Some(State::Walking), Event::WalkStarted { direction, distance }) = (changed, &event) {
                        plan = Some(WalkPlan { direction: *direction, distance: *distance });
                    }
                    changed
                }
                Err(RecvTimeoutError::Timeout) => {
                    let changed = character.tick(now);
                    if changed.is_none() {
                        // Con el chat abierto no pasea: la ventana del chat no la seguiría.
                        let enabled = flags.wander.load(Ordering::Relaxed) && !flags.chat_open.load(Ordering::Relaxed);
                        if let Some(event) = wander.poll(character.state(), now, enabled) {
                            bus.publish(event);
                        }
                    }
                    changed
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };

            if let Some(plan) = plan {
                cancel.store(false, Ordering::Relaxed);
                let _ = walk_tx.send(plan);
            }
            if let Some(state) = changed {
                flags.walking.store(state == State::Walking, Ordering::Relaxed);
                flags.sleeping.store(state == State::Sleeping, Ordering::Relaxed);
                if state != State::Walking {
                    cancel.store(true, Ordering::Relaxed);
                }
                #[cfg(debug_assertions)]
                println!("estado: {}", state.as_str());
                let _ = app.emit("character-state", state.as_str());
            }
        }
    });
}

/// Mueve la ventana de la mascota a lo largo del borde inferior mientras dure cada paseo.
fn spawn_walker(app: AppHandle, plans: Receiver<WalkPlan>, cancel: Arc<AtomicBool>, bus: Arc<EventBus>) {
    std::thread::spawn(move || {
        while let Ok(plan) = plans.recv() {
            walk(&app, &plan, &cancel);
            bus.publish(Event::WalkFinished);
        }
    });
}

fn walk(app: &AppHandle, plan: &WalkPlan, cancel: &AtomicBool) {
    let Some(window) = app.get_webview_window("main") else { return };
    let (Ok(Some(monitor)), Ok(position), Ok(size)) = (window.current_monitor(), window.outer_position(), window.outer_size())
    else {
        return;
    };
    let area = monitor.work_area();
    let bounds = WalkBounds {
        left: area.position.x + EDGE_MARGIN,
        right: area.position.x + area.size.width as i32 - size.width as i32 - EDGE_MARGIN,
    };
    let direction = effective_direction(position.x, plan.direction, bounds);
    let _ = app.emit("pet-facing", Facing::from_direction(direction).as_str());

    let started = Instant::now();
    let distance = plan.distance as i32;
    while !cancel.load(Ordering::Relaxed) {
        let travelled = ((started.elapsed().as_secs_f64() * WALK_SPEED) as i32).min(distance);
        let (x, blocked) = position_after(position.x, direction, travelled, bounds);
        let _ = window.set_position(PhysicalPosition::new(x, position.y));
        if travelled >= distance || blocked {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Gira la mascota hacia el cursor cuando lo tiene cerca (despierta y sin caminar).
fn spawn_cursor_watcher(app: AppHandle, flags: Arc<BehaviorFlags>) {
    std::thread::spawn(move || {
        let mut last: Option<Facing> = None;
        loop {
            std::thread::sleep(Duration::from_millis(120));
            if !flags.look_at_cursor.load(Ordering::Relaxed)
                || flags.walking.load(Ordering::Relaxed)
                || flags.sleeping.load(Ordering::Relaxed)
            {
                last = None;
                continue;
            }
            let Some(pet) = app.get_webview_window("main") else { continue };
            let (Ok(cursor), Ok(position), Ok(size)) = (pet.cursor_position(), pet.outer_position(), pet.outer_size()) else {
                continue;
            };
            let dx = cursor.x - (position.x as f64 + size.width as f64 / 2.0);
            let dy = cursor.y - (position.y as f64 + size.height as f64 / 2.0);
            if let Some(facing) = facing_towards(dx, dy, LOOK_RADIUS, LOOK_DEAD_ZONE) {
                if last != Some(facing) {
                    last = Some(facing);
                    let _ = app.emit("pet-facing", facing.as_str());
                }
            }
        }
    });
}