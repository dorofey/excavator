//! Favorite navigation and management. All preferences I/O runs off the event loop.
use super::{
    modal::{Tone, panel},
    view::display_os,
};
use crate::{domain::Location, persistence};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Paragraph,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

/// Run only on the preferences worker. Removing stale shortcuts needs no filesystem check.
fn checked_mutation<T>(
    path: &std::path::Path,
    add: bool,
    save: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if add {
        let metadata = std::fs::metadata(path)
            .map_err(|error| format!("Cannot inspect favorite folder: {error}"))?;
        if !metadata.is_dir() {
            return Err("Only existing local folders can be saved as favorites".into());
        }
    }
    save()
}

pub struct Favorites {
    active: bool,
    paths: Vec<PathBuf>,
    index: usize,
    worker: Option<Receiver<(Vec<PathBuf>, Option<String>)>>,
    error: Option<String>,
    navigate: Option<Location>,
    dirty: bool,
    confirm_remove: Option<PathBuf>,
    remove_mode: bool,
    mutation: Option<bool>,
    notice: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    #[test]
    fn held_enter_cannot_approve_favorite_removal() {
        let mut favorites = Favorites::new();
        favorites.active = true;
        favorites.remove_mode = true;
        favorites.paths = vec![PathBuf::from("/tmp/captured")];
        favorites.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(favorites.confirm_remove.is_some());
        let mut repeat = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        repeat.kind = KeyEventKind::Repeat;
        favorites.key(repeat);
        favorites.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(favorites.confirm_remove.is_some());
        assert!(favorites.worker.is_none());
    }

    #[test]
    fn additions_reject_missing_and_regular_files_without_changing_preferences() {
        let dir = std::env::temp_dir().join(format!(
            "excavator-favorite-folder-check-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("preferences.json");
        persistence::save_to_path(&settings, &persistence::Preferences::default()).unwrap();
        let before = std::fs::read(&settings).unwrap();
        let regular = dir.join("regular-file");
        std::fs::write(&regular, b"file").unwrap();
        for path in [dir.join("missing"), regular] {
            assert!(
                checked_mutation(&path, true, || persistence::update_favorite_at(
                    &settings, &path, true
                ))
                .is_err()
            );
            assert_eq!(std::fs::read(&settings).unwrap(), before);
        }
        let stale = dir.join("stale-folder");
        persistence::update_favorite_at(&settings, &stale, true).unwrap();
        assert!(
            checked_mutation(&stale, false, || persistence::update_favorite_at(
                &settings, &stale, false
            ))
            .unwrap()
            .1
        );
        let link = dir.join("directory-link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(
            checked_mutation(&link, true, || persistence::update_favorite_at(
                &settings, &link, true
            ))
            .unwrap()
            .1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelled_load_never_navigates_and_paths_keep_original_bytes() {
        let mut favorites = Favorites::new();
        let (tx, rx) = mpsc::sync_channel(1);
        favorites.worker = Some(rx);
        favorites.active = true;
        favorites.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let path = PathBuf::from(OsString::from_vec(b"/tmp/favorite-\xff".to_vec()));
        tx.send((vec![path.clone()], None)).unwrap();
        assert!(favorites.poll().0.is_none());
        assert!(!favorites.active());
        favorites.active = true;
        favorites.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(favorites.poll().0, Some(Location::Local(path)));
        assert!(!favorites.active());
    }

    #[test]
    fn removal_confirmation_captures_path_and_escape_keeps_favorite() {
        let mut favorites = Favorites::new();
        favorites.active = true;
        let original = PathBuf::from("/tmp/original");
        favorites.paths = vec![original.clone(), PathBuf::from("/tmp/second")];
        favorites.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(favorites.confirm_remove, Some(original.clone()));
        favorites.index = 1;
        favorites.paths.swap(0, 1);
        assert_eq!(favorites.confirm_remove, Some(original.clone()));
        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();
        terminal.draw(|frame| favorites.render(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("/tmp/original"));
        super::super::modal::capture("favorite-remove", terminal.backend().buffer());
        favorites.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(favorites.confirm_remove.is_none());
        assert!(favorites.paths.contains(&original));
        assert!(favorites.worker.is_none());
    }

    #[test]
    fn reload_preserves_selected_path_and_cancelled_save_does_not_reopen() {
        let mut favorites = Favorites::new();
        let selected = PathBuf::from("/tmp/selected");
        favorites.paths = vec![PathBuf::from("/tmp/old"), selected.clone()];
        favorites.index = 1;
        let (tx, rx) = mpsc::sync_channel(1);
        favorites.worker = Some(rx);
        favorites.mutation = Some(true);
        favorites.active = true;
        favorites.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        tx.send((vec![selected.clone(), PathBuf::from("/tmp/new")], None))
            .unwrap();
        let (_, notice, _) = favorites.poll();
        assert!(!favorites.active());
        assert_eq!(favorites.paths[favorites.index], selected);
        assert_eq!(notice.as_deref(), Some("Folder saved as a favorite"));
    }

    #[test]
    fn failed_mutation_preserves_loaded_favorites_and_surfaces_error() {
        let mut favorites = Favorites::new();
        favorites.paths = vec![PathBuf::from("/tmp/kept")];
        let (tx, rx) = mpsc::sync_channel(1);
        favorites.worker = Some(rx);
        favorites.mutation = Some(false);
        tx.send((vec![], Some("Cannot save preferences".into())))
            .unwrap();
        let (_, notice, _) = favorites.poll();
        assert_eq!(notice.as_deref(), Some("Cannot save preferences"));
        assert_eq!(favorites.paths, vec![PathBuf::from("/tmp/kept")]);
    }

    #[test]
    fn narrow_picker_keeps_selection_visible_and_surfaces_load_warning() {
        let mut favorites = Favorites::new();
        favorites.active = true;
        favorites.paths = (0..15)
            .map(|i| PathBuf::from(format!("/tmp/favorite-{i}")))
            .collect();
        favorites.index = 14;
        favorites.error = Some("Cannot read preferences".into());
        let mut terminal = Terminal::new(TestBackend::new(44, 12)).unwrap();
        terminal.draw(|frame| favorites.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("favorite-14"));
        assert!(text.contains("Cannot read preferences"));
        super::super::modal::capture("favorites-narrow", buffer);
        favorites.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(favorites.index, 13);
    }
}

impl Default for Favorites {
    fn default() -> Self {
        Self::new()
    }
}

impl Favorites {
    pub fn new() -> Self {
        Self {
            active: false,
            paths: vec![],
            index: 0,
            worker: None,
            error: None,
            navigate: None,
            dirty: false,
            confirm_remove: None,
            remove_mode: false,
            mutation: None,
            notice: None,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn open(&mut self) {
        self.active = true;
        self.confirm_remove = None;
        self.remove_mode = false;
        self.dirty = true;
        if self.worker.is_some() {
            return;
        }
        self.error = None;
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        if std::thread::Builder::new()
            .name("tui-favorites".into())
            .spawn(move || {
                let (preferences, error) = persistence::load();
                let mut paths = Vec::new();
                for path in preferences.favorites {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
                let _ = tx.send((paths, error));
            })
            .is_err()
        {
            self.worker = None;
            self.error = Some("Cannot start favorites worker".into());
        }
    }

    pub fn request_remove(&mut self) {
        self.open();
        self.remove_mode = true;
    }

    pub fn add_current(&mut self, location: &Location) {
        match location {
            Location::Local(path) => self.mutate(path.clone(), true),
            _ => {
                self.notice = Some("Only local folders can be saved as favorites".into());
                self.dirty = true;
            }
        }
    }

    fn mutate(&mut self, path: PathBuf, add: bool) {
        if self.worker.is_some() {
            self.notice = Some("Favorites are busy; try again after loading or saving".into());
            self.dirty = true;
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        self.mutation = Some(add);
        self.error = None;
        self.dirty = true;
        if std::thread::Builder::new()
            .name("tui-favorite-save".into())
            .spawn(move || {
                let result =
                    checked_mutation(&path, add, || persistence::update_favorite(&path, add));
                let value = match result {
                    Ok((paths, _)) => (paths, None),
                    Err(error) => (vec![], Some(error)),
                };
                let _ = tx.send(value);
            })
            .is_err()
        {
            self.worker = None;
            self.mutation = None;
            self.notice = Some("Cannot start favorites save worker".into());
        }
    }

    pub fn poll(&mut self) -> (Option<Location>, Option<String>, bool) {
        let result = self.worker.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some((vec![], Some("Favorites worker stopped".into())))
            }
        });
        if let Some((paths, error)) = result {
            self.worker = None;
            let selected = self.paths.get(self.index).cloned();
            let mutation = self.mutation.take();
            if mutation.is_none() || error.is_none() {
                self.paths = paths;
                if let Some(path) = selected {
                    if let Some(index) = self.paths.iter().position(|candidate| candidate == &path)
                    {
                        self.index = index;
                    }
                }
            }
            if let Some(add) = mutation {
                self.notice = Some(error.clone().unwrap_or_else(|| {
                    if add {
                        "Folder saved as a favorite".into()
                    } else {
                        "Favorite removed".into()
                    }
                }));
            }
            self.error = error;
            self.index = self.index.min(self.paths.len().saturating_sub(1));
            self.dirty = true;
        }
        (
            self.navigate.take(),
            self.notice.take(),
            std::mem::take(&mut self.dirty),
        )
    }

    pub fn key(&mut self, key: KeyEvent) {
        if !self.active || !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        if matches!(
            key.code,
            KeyCode::Enter | KeyCode::Esc | KeyCode::Delete | KeyCode::Char('d')
        ) && (key.kind != KeyEventKind::Press
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER))
        {
            return;
        }
        if self.confirm_remove.is_some() {
            match key.code {
                KeyCode::Esc => self.confirm_remove = None,
                KeyCode::Enter => {
                    let path = self.confirm_remove.take().unwrap();
                    self.mutate(path, false);
                }
                _ => {}
            }
            self.dirty = true;
            return;
        }
        if key.code == KeyCode::Esc {
            self.active = false;
        } else if self.worker.is_none()
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.index = self.index.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    self.index = (self.index + 1).min(self.paths.len().saturating_sub(1))
                }
                KeyCode::Delete | KeyCode::Char('d') => {
                    self.confirm_remove = self.paths.get(self.index).cloned();
                }
                KeyCode::Enter => {
                    if let Some(path) = self.paths.get(self.index) {
                        if self.remove_mode {
                            self.confirm_remove = Some(path.clone());
                        } else {
                            self.navigate = Some(Location::Local(path.clone()));
                            self.active = false;
                        }
                    }
                }
                _ => {}
            }
        }
        self.dirty = true;
    }

    pub fn render(&self, frame: &mut Frame) {
        if !self.active {
            return;
        }
        if let Some(path) = &self.confirm_remove {
            let body = panel(
                frame,
                "Remove favorite?",
                84,
                4,
                "Enter remove · Esc cancel",
                Tone::Normal,
            );
            frame.render_widget(
                Paragraph::new(format!(
                    "Remove this saved shortcut?\n{}\nThe folder and its files remain unchanged.",
                    display_os(path.as_os_str())
                ))
                .wrap(ratatui::widgets::Wrap { trim: false }),
                body,
            );
            return;
        }
        let height = (self.paths.len().min(10) as u16).saturating_add(2).max(3);
        let body = panel(
            frame,
            "Favorites",
            84,
            height,
            if self.remove_mode {
                "↑↓ choose · Enter review removal · Esc cancel"
            } else {
                "↑↓ choose · Enter open · d remove · Esc cancel"
            },
            Tone::Normal,
        );
        if self.worker.is_some() {
            frame.render_widget(
                Paragraph::new(if self.mutation.is_some() {
                    "Saving favorites…"
                } else {
                    "Loading saved favorites…"
                }),
                body,
            );
            return;
        }
        let heading = self
            .error
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} saved local favorites", self.paths.len()));
        frame.render_widget(
            Paragraph::new(heading).style(Style::default().fg(if self.error.is_some() {
                Color::Red
            } else {
                Color::DarkGray
            })),
            Rect::new(body.x, body.y, body.width, body.height.min(1)),
        );
        let rows = body.height.saturating_sub(2) as usize;
        if self.paths.is_empty() && body.height > 2 {
            frame.render_widget(
                Paragraph::new(
                    "No saved favorites. Use Add current folder in the command palette.",
                ),
                Rect::new(body.x, body.y + 2, body.width, 1),
            );
        }
        let start = self.index.saturating_sub(rows.saturating_sub(1));
        for (offset, (index, path)) in self
            .paths
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .enumerate()
        {
            let selected = index == self.index;
            let text = format!(
                "{} {}",
                if selected { "›" } else { " " },
                display_os(path.as_os_str())
            );
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            frame.render_widget(
                Paragraph::new(text).style(style),
                Rect::new(body.x, body.y + 2 + offset as u16, body.width, 1),
            );
        }
    }
}
