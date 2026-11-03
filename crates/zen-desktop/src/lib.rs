mod app_catalog;
mod applications;
mod args;
mod browse_tools;
mod clock;
mod delete_tools;
mod fs_tools;
pub mod input;
mod input_tools;
mod locations;
pub mod media_session;
pub mod listening;
pub mod memory;
pub mod mood;
mod media_tools;
mod move_tools;
mod music_tools;
mod movement;
mod names;
mod organize;
pub mod power;
mod uia;
mod powershell;
mod read_tools;
mod reminders;
mod screen;
mod screen_tools;
pub mod sound;
pub mod system_events;
pub mod usage;
mod system_tools;
#[cfg(test)]
mod testutil;
mod web_tools;

use std::path::PathBuf;
use std::sync::Arc;

use zen_core::tools::ToolRegistry;

pub use app_catalog::{is_blocked as is_blocked_app, AppCatalog, BuiltinStatus, CustomApp, DiscoveredApp};
pub use input_tools::TOOL_NAMES as COMPUTER_CONTROL_TOOLS;
pub use clock::{describe_span, play_alert_sound, unix_now, LocalNow};
pub use locations::{Location, Locations};
pub use memory::Memory;
pub use reminders::Reminders;
pub use web_tools::open_url;
pub use movement::{effective_direction, facing_towards, position_after, Facing, WalkBounds};

/// Herramientas disponibles. `data_dir` es donde se guarda el registro para deshacer la última organización;
/// `reminders` es el almacén que también consulta el hilo que avisa cuando vence uno.
pub fn register_default_tools(
    registry: &mut ToolRegistry,
    locations: Arc<Locations>,
    data_dir: PathBuf,
    reminders: Arc<Reminders>,
    apps: Arc<AppCatalog>,
    memory: Arc<Memory>,
) {
    let journal = organize::Journal::new(data_dir);
    registry.register(fs_tools::CreateFolder::new(locations.clone()));
    registry.register(fs_tools::CreateFile::new(locations.clone()));
    registry.register(browse_tools::ListFiles::new(locations.clone()));
    registry.register(read_tools::ReadFile::new(locations.clone()));
    registry.register(read_tools::SearchFiles::new(locations.clone()));
    registry.register(move_tools::MoveFile::new(locations.clone()));
    registry.register(delete_tools::DeleteFile::new(locations.clone()));
    registry.register(organize::OrganizeFolder::new(locations.clone(), journal.clone()));
    registry.register(organize::UndoOrganize::new(locations, journal));
    registry.register(app_catalog::OpenApplication::new(apps));
    registry.register(web_tools::OpenWebsite);
    registry.register(web_tools::SearchMusic);
    registry.register(music_tools::PlayMusic);
    registry.register(media_tools::MediaControl);
    registry.register(screen_tools::LookAtScreen::new());
    registry.register(input_tools::ListWindows);
    registry.register(input_tools::ListControls);
    registry.register(input_tools::TypeText::new());
    registry.register(input_tools::PressKeys::new());
    registry.register(input_tools::ClickControl::new());
    registry.register(system_tools::SystemInfo);
    registry.register(system_tools::SetBrightness);
    registry.register(system_tools::ReadClipboard);
    registry.register(system_tools::WriteClipboard);
    registry.register(reminders::SetReminder::new(reminders.clone()));
    registry.register(reminders::ListReminders::new(reminders.clone()));
    registry.register(reminders::CancelReminder::new(reminders));
    registry.register(memory::Remember(memory.clone()));
    registry.register(memory::Forget(memory));
}
