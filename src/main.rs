mod avatar;
mod config;
mod git;
mod ui;

use std::path::{Path, PathBuf};

fn main() -> iced::Result {
    let config = or_exit(config::Config::load());
    let mut state = config::State::load();
    let mut repositories = config::repositories(&config, &state);
    let mut changed = false;

    let known = state
        .last_opened
        .clone()
        .filter(|path| repositories.contains(path))
        .or_else(|| repositories.first().cloned());

    let (opened, remember) = match (std::env::args_os().nth(1), known) {
        (Some(argument), _) => {
            let path = PathBuf::from(argument);
            let root = or_exit(
                git::read::root(&path).map_err(|error| format!("{}: {error}", path.display())),
            );
            (root, true)
        }
        (None, Some(path)) => (path, false),
        (None, None) => match git::read::root(Path::new(".")) {
            Ok(root) => (root, true),
            Err(_) => (PathBuf::from("."), false),
        },
    };

    if remember && !repositories.contains(&opened) {
        repositories.push(opened.clone());
        state.paths.push(opened.clone());
        changed = true;
    }
    if repositories.contains(&opened) && state.last_opened.as_ref() != Some(&opened) {
        state.last_opened = Some(opened.clone());
        changed = true;
    }
    if changed && let Err(error) = state.save() {
        eprintln!("gg: {error}");
    }

    ui::run(config, state, repositories, opened)
}

fn or_exit<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            eprintln!("gg: {error}");
            std::process::exit(1)
        }
    }
}
