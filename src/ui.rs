use std::{
    collections::HashMap,
    env,
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{
    launch::{self, AuthOperation, AuthStatus, Tool},
    profile::{Profile, Store},
    settings, shared,
};

const DITTO_PURPLE: Color = Color::Rgb(190, 134, 255);
const CLAUDE_ORANGE: Color = Color::Rgb(222, 133, 93);
const CODEX_GREEN: Color = Color::Rgb(104, 201, 154);
const FX_MAGENTA: Color = Color::Rgb(236, 72, 153);
const OPENCODE_CYAN: Color = Color::Rgb(103, 199, 209);
const OMP_BLUE: Color = Color::Rgb(96, 165, 250);
const PRIME_PURPLE: Color = Color::Rgb(168, 85, 247);

/// Width reserved for the tool name so the status and path columns line up.
const TOOL_COLUMN: usize = 13;
const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠇"];
/// How long the loop waits for input before looking for finished probes.
const TICK: Duration = Duration::from_millis(110);
/// Below this the panes cannot show a usable amount of the profile.
const MINIMUM_WIDTH: u16 = 68;
const MINIMUM_HEIGHT: u16 = 26;
/// The width at which every profile shortcut fits on one footer row.
const WIDE_FOOTER_WIDTH: u16 = 88;
/// Marks the profile that commands use when they omit a profile name.
const DEFAULT_MARK: &str = "★";

/// A footer entry: the key, what it does, and the colour of the key.
type Shortcut = (&'static str, &'static str, Color);

const SELECT: Shortcut = ("↑↓", "select", DITTO_PURPLE);
const NEW: Shortcut = ("n", "new", Color::Gray);
const RENAME: Shortcut = ("e", "rename", Color::Gray);
const DEFAULT: Shortcut = ("d", "default", Color::Gray);
const SIGN_IN: Shortcut = ("l", "sign in", Color::Gray);
const SIGN_OUT: Shortcut = ("L", "sign out", Color::Gray);
const REFRESH: Shortcut = ("r", "refresh", Color::Gray);
const QUIT: Shortcut = ("q", "quit", Color::Gray);

const LAUNCH_CLAUDE: Shortcut = ("c", "Claude Code", CLAUDE_ORANGE);
const LAUNCH_CODEX: Shortcut = ("x", "Codex", CODEX_GREEN);
const LAUNCH_FX: Shortcut = ("f", "fx", FX_MAGENTA);
const LAUNCH_OPENCODE: Shortcut = ("o", "opencode", OPENCODE_CYAN);
const LAUNCH_OMP: Shortcut = ("p", "OMP", OMP_BLUE);
const LAUNCH_PRIME: Shortcut = ("a", "Prime", PRIME_PURPLE);
const LAUNCH_PI: Shortcut = ("i", "Pi", DITTO_PURPLE);
const LAUNCH_MORE: Shortcut = ("⏎", "any tool", DITTO_PURPLE);
const TOOL_SHORTCUTS: [Shortcut; 8] = [
    LAUNCH_CLAUDE,
    LAUNCH_CODEX,
    LAUNCH_FX,
    LAUNCH_OPENCODE,
    LAUNCH_OMP,
    LAUNCH_PRIME,
    LAUNCH_PI,
    LAUNCH_MORE,
];
/// The same eight over two rows, for a terminal too narrow to show one.
const NARROW_TOOL_ROWS: [&[Shortcut]; 2] = [
    &[LAUNCH_CLAUDE, LAUNCH_CODEX, LAUNCH_FX, LAUNCH_OPENCODE],
    &[LAUNCH_OMP, LAUNCH_PRIME, LAUNCH_PI, LAUNCH_MORE],
];
const AUTH_SHORTCUTS: [Shortcut; 5] = [
    ("c", "Claude Code", CLAUDE_ORANGE),
    ("x", "Codex", CODEX_GREEN),
    ("f", "fx", FX_MAGENTA),
    ("o", "opencode", OPENCODE_CYAN),
    ("a", "Prime Agent", PRIME_PURPLE),
];
const WIDE_SHORTCUT_ROW: [Shortcut; 8] = [
    SELECT, NEW, RENAME, DEFAULT, SIGN_IN, SIGN_OUT, REFRESH, QUIT,
];
const NARROW_SHORTCUT_ROWS: [&[Shortcut]; 2] = [
    &[SELECT, NEW, RENAME, DEFAULT],
    &[SIGN_IN, SIGN_OUT, REFRESH, QUIT],
];

pub enum UiAction {
    Launch {
        tool: Tool,
        profile: Profile,
    },
    Authenticate {
        operation: AuthOperation,
        tool: Tool,
        profile: Profile,
    },
}

enum Mode {
    Browsing,
    Creating {
        input: String,
        error: Option<String>,
    },
    Renaming {
        original: String,
        input: String,
        error: Option<String>,
    },
    Notice {
        title: &'static str,
        message: String,
    },
    ChoosingTool {
        operation: AuthOperation,
    },
    /// Picking any installed tool to launch: too many for a letter each, so
    /// the list is typed into instead.
    Launching {
        filter: String,
        selected: usize,
    },
    ConfirmingLogout {
        tool: Tool,
    },
}

/// Sign-in state for one profile. `None` means the probe is still running, so
/// the interface can say "checking" instead of guessing.
#[derive(Clone, Copy)]
struct ProfileAuth {
    generation: u64,
    /// Indexed by [`Tool::index`]: a field per tool stopped being possible
    /// when tools became a table.
    statuses: [Option<AuthStatus>; Tool::ALL.len()],
}

impl Default for ProfileAuth {
    fn default() -> Self {
        Self {
            generation: 0,
            statuses: [None; Tool::ALL.len()],
        }
    }
}

impl ProfileAuth {
    fn get(&self, tool: Tool) -> Option<AuthStatus> {
        self.statuses[tool.index()]
    }

    fn set(&mut self, tool: Tool, status: AuthStatus) {
        self.statuses[tool.index()] = Some(status);
    }

    fn pending(&self) -> bool {
        Tool::ALL.iter().any(|tool| self.get(*tool).is_none())
    }
}

/// A finished sign-in probe. The generation lets a refresh discard answers
/// that were already in flight when it started.
struct Probe {
    profile: String,
    generation: u64,
    tool: Tool,
    status: AuthStatus,
}

struct App<'a> {
    store: &'a Store,
    profiles: Vec<Profile>,
    selected: usize,
    mode: Mode,
    auth: HashMap<String, ProfileAuth>,
    generation: u64,
    sender: Sender<Probe>,
    receiver: Receiver<Probe>,
    spinner: usize,
    has_auth_environment: bool,
    default_profile: Option<String>,
    /// Every tool that is on `PATH`, found once: `PATH` does not change while
    /// the picker is open, and thirty-six lookups per redraw would show.
    installed: Vec<Tool>,
}

impl<'a> App<'a> {
    fn new(
        store: &'a Store,
        profiles: Vec<Profile>,
        initial_profile: Option<&str>,
        default_profile: Option<String>,
    ) -> Self {
        let selected = initial_profile
            .and_then(|name| profiles.iter().position(|profile| profile.name == name))
            .unwrap_or(0);
        let (sender, receiver) = mpsc::channel();
        let mut app = Self {
            store,
            profiles,
            selected,
            mode: Mode::Browsing,
            auth: HashMap::new(),
            generation: 0,
            sender,
            receiver,
            spinner: 0,
            has_auth_environment: auth_environment_is_set(),
            default_profile,
            installed: Tool::ALL
                .into_iter()
                .filter(|tool| tool.installed())
                .collect(),
        };
        app.probe_selected();
        app
    }

    fn selected_profile(&self) -> &Profile {
        &self.profiles[self.selected]
    }

    fn selected_auth(&self) -> ProfileAuth {
        self.auth
            .get(&self.selected_profile().name)
            .copied()
            .unwrap_or_default()
    }

    fn move_to(&mut self, index: usize) {
        let last = self.profiles.len().saturating_sub(1);
        self.selected = index.min(last);
        if !self.auth.contains_key(&self.selected_profile().name) {
            self.probe_selected();
        }
    }

    /// Asks each CLI about the selected profile on its own thread. The probes
    /// spawn other programs, so running them inline would freeze the list
    /// every time the cursor moves.
    fn probe_selected(&mut self) {
        let profile = self.selected_profile().clone();
        self.generation += 1;
        let generation = self.generation;
        self.auth.insert(
            profile.name.clone(),
            ProfileAuth {
                generation,
                ..ProfileAuth::default()
            },
        );

        for tool in Tool::ALL {
            let sender = self.sender.clone();
            let profile = profile.clone();
            thread::spawn(move || {
                let status = launch::auth_status(tool, &profile);
                let _ = sender.send(Probe {
                    profile: profile.name,
                    generation,
                    tool,
                    status,
                });
            });
        }
    }

    /// Collects finished probes. Returns whether anything on screen changed.
    fn collect_probes(&mut self) -> bool {
        let mut changed = false;
        while let Ok(probe) = self.receiver.try_recv() {
            if let Some(auth) = self.auth.get_mut(&probe.profile)
                && auth.generation == probe.generation
            {
                auth.set(probe.tool, probe.status);
                changed = true;
            }
        }
        changed
    }

    fn waiting_on_probes(&self) -> bool {
        self.selected_auth().pending()
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Action> {
        if key.kind != KeyEventKind::Press {
            return Ok(Action::Continue);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(Action::Quit);
        }
        // Held modifiers arrive as ordinary characters, so without this Ctrl-H
        // would type an "h" into a name and Ctrl-C would launch Claude Code.
        // Shift is exempt: it is how the sign-out shortcut is typed.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Ok(Action::Continue);
        }

        match &mut self.mode {
            Mode::Browsing => Ok(match key.code {
                KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_to(self.selected.saturating_sub(1));
                    Action::Continue
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_to(self.selected + 1);
                    Action::Continue
                }
                KeyCode::Home => {
                    self.move_to(0);
                    Action::Continue
                }
                KeyCode::End => {
                    self.move_to(usize::MAX);
                    Action::Continue
                }
                KeyCode::Char('n') => {
                    self.mode = Mode::Creating {
                        input: String::new(),
                        error: None,
                    };
                    Action::Continue
                }
                KeyCode::Char('e') => {
                    if self.selected_profile().managed {
                        self.mode = Mode::Renaming {
                            original: self.selected_profile().name.clone(),
                            input: String::new(),
                            error: None,
                        };
                    } else {
                        self.mode = Mode::Notice {
                            title: " Cannot rename ",
                            message: "The default profile represents your existing setup and cannot be renamed."
                                .to_owned(),
                        };
                    }
                    Action::Continue
                }
                KeyCode::Char('l') => {
                    self.mode = Mode::ChoosingTool {
                        operation: AuthOperation::Login,
                    };
                    Action::Continue
                }
                KeyCode::Char('L') => {
                    self.mode = Mode::ChoosingTool {
                        operation: AuthOperation::Logout,
                    };
                    Action::Continue
                }
                KeyCode::Char('d') => {
                    self.toggle_default();
                    Action::Continue
                }
                KeyCode::Char('r') => {
                    self.probe_selected();
                    Action::Continue
                }
                KeyCode::Char('c') => Action::Launch(Tool::Claude),
                KeyCode::Char('x') => Action::Launch(Tool::Codex),
                KeyCode::Char('f') => Action::Launch(Tool::Fx),
                KeyCode::Char('o') => Action::Launch(Tool::Opencode),
                KeyCode::Char('p') => Action::Launch(Tool::Omp),
                KeyCode::Char('a') => Action::Launch(Tool::PrimeAgent),
                KeyCode::Char('i') => Action::Launch(Tool::Pi),
                KeyCode::Enter | KeyCode::Char('t') => {
                    self.mode = Mode::Launching {
                        filter: String::new(),
                        selected: 0,
                    };
                    Action::Continue
                }
                _ => Action::Continue,
            }),
            Mode::Creating { input, error } => match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Browsing;
                    Ok(Action::Continue)
                }
                KeyCode::Enter => match self.store.create_profile(input) {
                    Ok(profile) => {
                        settings::seed(self.store, &profile);
                        shared::seed(self.store, &profile);
                        self.select_after_change(&profile.name)?;
                        Ok(Action::Continue)
                    }
                    Err(create_error) => {
                        *error = Some(create_error.to_string());
                        Ok(Action::Continue)
                    }
                },
                KeyCode::Backspace => {
                    input.pop();
                    *error = None;
                    Ok(Action::Continue)
                }
                KeyCode::Char(character) if input.len() < 32 => {
                    input.push(character);
                    *error = None;
                    Ok(Action::Continue)
                }
                _ => Ok(Action::Continue),
            },
            Mode::Renaming {
                original,
                input,
                error,
            } => match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Browsing;
                    Ok(Action::Continue)
                }
                KeyCode::Enter => {
                    let original = original.clone();
                    let signs_out = rename_signs_out(&self.auth, &original);
                    match self.store.rename_profile(&original, input) {
                        Ok(profile) => {
                            self.auth.remove(&original);
                            self.select_after_change(&profile.name)?;
                            if signs_out {
                                self.mode = Mode::Notice {
                                    title: " Claude Code signed out ",
                                    message: format!(
                                        "Claude Code ties its credentials to the profile \
                                         directory, which the rename moved. Press l to sign \
                                         '{}' back in.",
                                        profile.name
                                    ),
                                };
                            }
                            Ok(Action::Continue)
                        }
                        Err(rename_error) => {
                            if let Mode::Renaming { error, .. } = &mut self.mode {
                                *error = Some(rename_error.to_string());
                            }
                            Ok(Action::Continue)
                        }
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    *error = None;
                    Ok(Action::Continue)
                }
                KeyCode::Char(character) if input.len() < 32 => {
                    input.push(character);
                    *error = None;
                    Ok(Action::Continue)
                }
                _ => Ok(Action::Continue),
            },
            Mode::Notice { .. } => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                    self.mode = Mode::Browsing;
                    Ok(Action::Continue)
                }
                _ => Ok(Action::Continue),
            },
            Mode::Launching { filter, selected } => {
                let candidates = launchable(&self.installed, filter);
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browsing,
                    KeyCode::Enter => {
                        if let Some(tool) = candidates.get(*selected) {
                            return Ok(Action::Launch(*tool));
                        }
                    }
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => {
                        *selected = (*selected + 1).min(candidates.len().saturating_sub(1));
                    }
                    KeyCode::Backspace => {
                        filter.pop();
                        *selected = 0;
                    }
                    // Letters narrow the list rather than launching, so the
                    // profile shortcuts cannot fire from inside it.
                    KeyCode::Char(character) if filter.len() < 32 => {
                        filter.push(character);
                        *selected = 0;
                    }
                    _ => {}
                }
                Ok(Action::Continue)
            }
            Mode::ChoosingTool { operation } => {
                let operation = *operation;
                let tool = match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Browsing;
                        return Ok(Action::Continue);
                    }
                    KeyCode::Char('c') => Tool::Claude,
                    KeyCode::Char('x') => Tool::Codex,
                    KeyCode::Char('f') => Tool::Fx,
                    KeyCode::Char('o') => Tool::Opencode,
                    KeyCode::Char('a') => Tool::PrimeAgent,
                    _ => return Ok(Action::Continue),
                };
                if operation == AuthOperation::Logout {
                    self.mode = Mode::ConfirmingLogout { tool };
                    Ok(Action::Continue)
                } else {
                    Ok(Action::Authenticate { operation, tool })
                }
            }
            Mode::ConfirmingLogout { tool } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => Ok(Action::Authenticate {
                    operation: AuthOperation::Logout,
                    tool: *tool,
                }),
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.mode = Mode::Browsing;
                    Ok(Action::Continue)
                }
                _ => Ok(Action::Continue),
            },
        }
    }

    fn is_default(&self, name: &str) -> bool {
        self.default_profile.as_deref() == Some(name)
    }

    /// Pins the selected profile so commands that omit a name use it, or
    /// releases it when it is already pinned. The state file is written now
    /// rather than on exit, so the pin survives a crash or a Ctrl-C.
    fn toggle_default(&mut self) {
        let name = self.selected_profile().name.clone();
        let pinned = (!self.is_default(&name)).then_some(name);

        match self.store.set_default_profile_name(pinned.as_deref()) {
            Ok(()) => self.default_profile = pinned,
            Err(error) => {
                self.mode = Mode::Notice {
                    title: " Cannot set default ",
                    message: format!("{error:#}"),
                };
            }
        }
    }

    /// Reloads the list after a create or rename and puts the cursor on the
    /// profile the change produced.
    fn select_after_change(&mut self, name: &str) -> Result<()> {
        self.profiles = self.store.list_profiles()?;
        // A rename rewrites the pin, so it is re-read rather than assumed.
        self.default_profile = self.store.default_profile_name()?;
        self.selected = self
            .profiles
            .iter()
            .position(|candidate| candidate.name == name)
            .unwrap_or(0);
        self.probe_selected();
        self.mode = Mode::Browsing;
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < MINIMUM_WIDTH || area.height < MINIMUM_HEIGHT {
            self.draw_too_small(frame, area);
            return;
        }

        // The profile shortcuts need a second row before they would be cut off.
        let narrow = area.width < WIDE_FOOTER_WIDTH;
        // Narrow mode splits both the tools and the actions over two rows.
        let footer_height = 4 + 2 * u16::from(narrow) + u16::from(self.has_auth_environment);
        let sections = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(footer_height),
        ])
        .split(area);

        self.draw_header(frame, sections[0]);
        self.draw_profiles(frame, sections[1]);
        self.draw_footer(frame, sections[2], narrow);
        self.draw_modal(frame, area);
    }

    fn draw_too_small(&self, frame: &mut Frame, area: Rect) {
        let message = Paragraph::new(vec![
            Line::styled("Ditto CLI", Style::new().fg(DITTO_PURPLE).bold()),
            Line::default(),
            Line::raw(format!(
                "Resize to at least {MINIMUM_WIDTH}×{MINIMUM_HEIGHT}."
            )),
            Line::styled(
                format!("This terminal is {}×{}.", area.width, area.height),
                Style::new().fg(Color::DarkGray),
            ),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
        frame.render_widget(Clear, area);
        frame.render_widget(message, centered_rect(90, 4.min(area.height), area));
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let header = Paragraph::new(Line::from(vec![
            Span::styled("Ditto CLI", Style::new().fg(DITTO_PURPLE).bold()),
            Span::styled(
                "  choose a profile, then Enter to pick a tool",
                Style::new().fg(Color::Gray),
            ),
        ]))
        .alignment(Alignment::Center)
        .block(Block::bordered().border_style(Style::new().fg(DITTO_PURPLE)));
        frame.render_widget(header, area);
    }

    fn draw_profiles(&self, frame: &mut Frame, area: Rect) {
        let columns = Layout::horizontal([Constraint::Length(26), Constraint::Min(30)]).split(area);
        let items = self.profiles.iter().map(|profile| {
            let suffix = if profile.managed { "" } else { "  existing" };
            let mark = if self.is_default(&profile.name) {
                format!("  {DEFAULT_MARK}")
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::raw(&profile.name),
                Span::styled(suffix, Style::new().fg(Color::DarkGray)),
                Span::styled(mark, Style::new().fg(Color::Yellow)),
            ]))
        });
        let profile_list = List::new(items)
            .block(Block::new().title(" Profiles ").borders(Borders::ALL))
            .highlight_symbol("› ")
            .highlight_style(
                Style::new()
                    .fg(Color::Black)
                    .bg(DITTO_PURPLE)
                    .add_modifier(Modifier::BOLD),
            );
        let mut list_state = ListState::default().with_selected(Some(self.selected));
        frame.render_stateful_widget(profile_list, columns[0], &mut list_state);

        // Deliberately unwrapped: a directory reflowed across three lines is
        // harder to read than one that is shortened to fit.
        let details = self.profile_details(columns[1].width.saturating_sub(2) as usize);
        frame.render_widget(
            Paragraph::new(details).block(
                Block::new()
                    .title(" Selected profile ")
                    .borders(Borders::ALL),
            ),
            columns[1],
        );
    }

    fn profile_details(&self, width: usize) -> Text<'static> {
        let profile = self.selected_profile();
        let auth = self.selected_auth();
        let home = self.store.user_home();
        let kind = if profile.managed {
            "Isolated profile"
        } else {
            "Your existing setup"
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(profile.name.clone(), Style::new().fg(DITTO_PURPLE).bold()),
                Span::styled(format!("  {kind}"), Style::new().fg(Color::DarkGray)),
            ]),
            Line::default(),
        ];
        if self.is_default(&profile.name) {
            lines.push(Line::styled(
                format!("{DEFAULT_MARK} Used when no profile is named"),
                Style::new().fg(Color::Yellow),
            ));
            lines.push(Line::default());
        }
        // The table's tools appear only when installed: the pane has room for
        // the handful a person has, not for every agent Ditto knows.
        let shown = Tool::ALL
            .into_iter()
            .filter(|tool| !matches!(tool, Tool::Generic(_)) || self.installed.contains(tool))
            .collect::<Vec<_>>();
        lines.push(Line::styled("Sign-in status", Style::new().bold()));
        lines.extend(
            shown
                .iter()
                .map(|&tool| status_row(tool, auth.get(tool), self.spinner)),
        );

        lines.push(Line::default());
        lines.push(Line::styled("Profile directories", Style::new().bold()));
        lines.extend(shown.iter().map(|&tool| {
            let path = match tool {
                Tool::Claude => profile.claude_home.clone(),
                Tool::Codex => profile.codex_home.clone(),
                Tool::Fx => profile.fx_dir(),
                Tool::Opencode => profile.opencode.data_dir(),
                Tool::Omp => profile.omp_home.clone(),
                Tool::PrimeAgent => profile.prime_agent_home.clone(),
                Tool::Pi => profile.pi_home.clone(),
                Tool::Generic(spec) => profile.tool_root(spec),
            };
            let path = shorten_home(&path, home);
            Line::from(vec![
                Span::styled(
                    format!("{:<TOOL_COLUMN$}", tool.label()),
                    Style::new().fg(tool_color(tool)),
                ),
                Span::styled(
                    truncate_start(&path, width.saturating_sub(TOOL_COLUMN)),
                    Style::new().fg(Color::DarkGray),
                ),
            ])
        }));

        if !profile.managed {
            lines.push(Line::default());
            lines.push(Line::styled(
                "Press n to create an isolated profile.",
                Style::new().fg(Color::DarkGray),
            ));
        }

        Text::from(lines)
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect, narrow: bool) {
        let mut lines = Vec::new();
        if narrow {
            lines.extend(NARROW_TOOL_ROWS.iter().map(|row| shortcut_line(row)));
            lines.extend(NARROW_SHORTCUT_ROWS.iter().map(|row| shortcut_line(row)));
        } else {
            lines.push(shortcut_line(&TOOL_SHORTCUTS));
            lines.push(shortcut_line(&WIDE_SHORTCUT_ROW));
        }
        if self.has_auth_environment {
            lines.push(Line::styled(
                "An API-key environment variable is set and may override the saved login.",
                Style::new().fg(Color::Yellow),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .block(Block::bordered().border_style(Style::new().fg(Color::DarkGray))),
            area,
        );
    }

    fn draw_modal(&self, frame: &mut Frame, area: Rect) {
        match &self.mode {
            Mode::Browsing => {}
            Mode::Creating { input, error } => {
                let mut lines = vec![
                    Line::raw("Use lowercase letters, numbers, '.', '-' or '_'."),
                    Line::styled(format!("> {input}"), Style::new().fg(DITTO_PURPLE).bold()),
                    Line::default(),
                    Line::styled(
                        "Enter create  ·  Esc cancel",
                        Style::new().fg(Color::DarkGray),
                    ),
                ];
                if let Some(error) = error {
                    lines[2] = Line::styled(error.clone(), Style::new().fg(Color::Red));
                }
                render_popup(frame, centered_rect(64, 8, area), " New profile ", lines);
            }
            Mode::Renaming {
                original,
                input,
                error,
            } => {
                let mut lines = vec![
                    Line::raw(format!("New name for '{original}':")),
                    Line::styled(format!("> {input}"), Style::new().fg(DITTO_PURPLE).bold()),
                    Line::default(),
                    Line::styled(
                        "Enter rename  ·  Esc cancel",
                        Style::new().fg(Color::DarkGray),
                    ),
                ];
                if let Some(error) = error {
                    lines[2] = Line::styled(error.clone(), Style::new().fg(Color::Red));
                } else if rename_signs_out(&self.auth, original) {
                    lines[2] = Line::styled(
                        "Claude Code will need a fresh sign-in afterwards.",
                        Style::new().fg(Color::Yellow),
                    );
                }
                render_popup(frame, centered_rect(64, 8, area), " Rename profile ", lines);
            }
            Mode::Notice { title, message } => {
                let lines = vec![
                    Line::raw(message.clone()),
                    Line::default(),
                    Line::styled("Enter or Esc close", Style::new().fg(Color::DarkGray)),
                ];
                render_popup(
                    frame,
                    centered_rect(64, notice_height(message, area), area),
                    title,
                    lines,
                );
            }
            Mode::ChoosingTool { operation } => {
                let lines = vec![
                    Line::raw(format!(
                        "{} to '{}' with:",
                        operation.label(),
                        self.selected_profile().name
                    )),
                    Line::default(),
                    shortcut_line(&AUTH_SHORTCUTS),
                    Line::default(),
                    // OMP and Pi are missing above on purpose: Ditto CLI can
                    // read their sign-in state but cannot open their commands.
                    Line::styled(
                        "OMP, Pi, and the other agents sign in and out from inside themselves.",
                        Style::new().fg(Color::DarkGray),
                    ),
                    Line::default(),
                    Line::styled("Esc cancel", Style::new().fg(Color::DarkGray)),
                ];
                render_popup(
                    frame,
                    centered_rect(90, 10, area),
                    &format!(" {} ", operation.label()),
                    lines,
                );
            }
            Mode::Launching { filter, selected } => {
                let auth = self.selected_auth();
                let candidates = launchable(&self.installed, filter);
                // Room for the prompt, the hint, the borders, and a blank line
                // either side of the list; the list scrolls inside what is left.
                let rows = usize::from(area.height.saturating_sub(8)).max(1);
                let first = selected
                    .saturating_sub(rows - 1)
                    .min(candidates.len().saturating_sub(rows));
                let mut lines = vec![
                    Line::from(vec![
                        Span::styled("› ", Style::new().fg(DITTO_PURPLE)),
                        Span::raw(filter.clone()),
                        Span::styled("▏", Style::new().fg(Color::DarkGray)),
                    ]),
                    Line::default(),
                ];
                if candidates.is_empty() {
                    lines.push(Line::styled(
                        if self.installed.is_empty() {
                            "No agent Ditto knows is installed."
                        } else {
                            "Nothing matches."
                        },
                        Style::new().fg(Color::DarkGray),
                    ));
                }
                lines.extend(candidates.iter().enumerate().skip(first).take(rows).map(
                    |(index, tool)| {
                        let row = status_row(*tool, auth.get(*tool), self.spinner);
                        if index == *selected {
                            row.style(Style::new().reversed())
                        } else {
                            row
                        }
                    },
                ));
                lines.push(Line::default());
                lines.push(Line::styled(
                    "type to filter · ↑↓ · Enter launches · Esc",
                    Style::new().fg(Color::DarkGray),
                ));
                let height = (lines.len() as u16 + 2).min(area.height);
                render_popup(
                    frame,
                    centered_rect(80, height, area),
                    &format!(" Launch in '{}' ", self.selected_profile().name),
                    lines,
                );
            }
            Mode::ConfirmingLogout { tool } => {
                let lines = vec![
                    Line::raw(format!(
                        "Sign out of {} for '{}'?",
                        tool.label(),
                        self.selected_profile().name
                    )),
                    Line::default(),
                    Line::styled(
                        "Enter or y confirm  ·  n cancel",
                        Style::new().fg(Color::Yellow),
                    ),
                ];
                render_popup(
                    frame,
                    centered_rect(62, 7, area),
                    " Confirm sign out ",
                    lines,
                );
            }
        }
    }
}

enum Action {
    Continue,
    Quit,
    Launch(Tool),
    Authenticate {
        operation: AuthOperation,
        tool: Tool,
    },
}

pub fn run(
    store: &Store,
    profiles: Vec<Profile>,
    initial_profile: Option<&str>,
    default_profile: Option<String>,
) -> Result<Option<UiAction>> {
    let app = App::new(store, profiles, initial_profile, default_profile);
    let mut terminal = ratatui::init();
    let guard = TerminalGuard;
    let result = run_loop(&mut terminal, app);
    drop(guard);
    result
}

fn run_loop(terminal: &mut DefaultTerminal, mut app: App<'_>) -> Result<Option<UiAction>> {
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| app.draw(frame))?;
            dirty = false;
        }

        if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) => {
                    match app.handle_key(key)? {
                        Action::Continue => {}
                        Action::Quit => return Ok(None),
                        Action::Launch(tool) => {
                            return Ok(Some(UiAction::Launch {
                                tool,
                                profile: app.selected_profile().clone(),
                            }));
                        }
                        Action::Authenticate { operation, tool } => {
                            return Ok(Some(UiAction::Authenticate {
                                operation,
                                tool,
                                profile: app.selected_profile().clone(),
                            }));
                        }
                    }
                    dirty = true;
                }
                Event::Resize(..) => dirty = true,
                _ => {}
            }
        } else if app.waiting_on_probes() {
            // Only animate while something is actually being waited on.
            app.spinner = app.spinner.wrapping_add(1);
            dirty = true;
        }

        if app.collect_probes() {
            dirty = true;
        }
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

fn tool_color(tool: Tool) -> Color {
    match tool {
        Tool::Claude => CLAUDE_ORANGE,
        Tool::Codex => CODEX_GREEN,
        Tool::Fx => FX_MAGENTA,
        Tool::Opencode => OPENCODE_CYAN,
        Tool::Omp => OMP_BLUE,
        Tool::PrimeAgent => PRIME_PURPLE,
        Tool::Pi => DITTO_PURPLE,
        Tool::Generic(_) => Color::Gray,
    }
}

/// One row of the tool table: the tool in its own colour, then its state.
fn tool_row(tool: Tool, symbol: &str, label: &str, state_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<TOOL_COLUMN$}", tool.label()),
            Style::new().fg(tool_color(tool)),
        ),
        Span::styled(format!("{symbol} {label}"), Style::new().fg(state_color)),
    ])
}

fn status_row(tool: Tool, status: Option<AuthStatus>, spinner: usize) -> Line<'static> {
    let (symbol, label, color) = match status {
        None => (
            SPINNER[spinner % SPINNER.len()],
            "Checking",
            Color::DarkGray,
        ),
        Some(AuthStatus::SignedIn) => ("●", "Signed in", Color::Green),
        Some(AuthStatus::SignedOut) => ("○", "Sign in required", Color::Yellow),
        // A CLI that is simply not installed is not an error worth alarming
        // about, so this stays quiet rather than red.
        Some(AuthStatus::Unavailable) => ("–", "Not available", Color::DarkGray),
    };
    tool_row(tool, symbol, label, color)
}

/// Paths are long enough to wrap the detail pane, so the home directory is
/// abbreviated the way a shell prompt would. The separator is the platform's
/// own, so what is shown reads as one path rather than two conventions spliced
/// together.
fn shorten_home(path: &Path, user_home: &Path) -> String {
    match path.strip_prefix(user_home) {
        Ok(relative) => format!("~{}{}", std::path::MAIN_SEPARATOR, relative.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Drops characters from the front of a path. The tail names the profile and
/// the tool, which is the part worth keeping; the head repeats on every row.
fn truncate_start(text: &str, budget: usize) -> String {
    let length = text.chars().count();
    if length <= budget {
        return text.to_owned();
    }
    if budget <= 1 {
        return "…".repeat(budget);
    }
    let mut truncated = String::from("…");
    truncated.extend(text.chars().skip(length - budget + 1));
    truncated
}

fn shortcut_line(shortcuts: &[(&str, &str, Color)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(shortcuts.len() * 3);
    for (index, (key, label, color)) in shortcuts.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Style::new().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            (*key).to_owned(),
            Style::new().fg(*color).bold(),
        ));
        spans.push(Span::raw(format!(" {label}")));
    }
    Line::from(spans)
}

fn render_popup(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::new()
                .title(title.to_owned())
                .borders(Borders::ALL)
                .border_style(Style::new().fg(DITTO_PURPLE)),
        ),
        area,
    );
}

/// The installed tools whose name contains what was typed, in the order they
/// are listed everywhere else. Case does not matter, and the key counts as
/// well as the label, so `cli` finds Gemini CLI and `kiro` finds `kiro-cli`.
fn launchable(installed: &[Tool], filter: &str) -> Vec<Tool> {
    let filter = filter.trim().to_lowercase();
    installed
        .iter()
        .copied()
        .filter(|tool| {
            filter.is_empty()
                || tool.label().to_lowercase().contains(&filter)
                || tool.key().contains(&filter)
        })
        .collect()
}

/// Whether renaming a profile costs it its Claude Code sign-in. Claude Code
/// stores credentials against the directory it was pointed at, and a rename
/// moves that directory, so a signed-in profile does not stay one. A probe
/// that has not answered yet is nothing to warn about rather than guessed at.
fn rename_signs_out(auth: &HashMap<String, ProfileAuth>, name: &str) -> bool {
    auth.get(name)
        .and_then(|auth| auth.get(Tool::Claude))
        .is_some_and(|status| status == AuthStatus::SignedIn)
}

/// Grows a notice to fit the message it carries. The popup wraps its text, so
/// a fixed height would clip anything longer than one line.
fn notice_height(message: &str, area: Rect) -> u16 {
    // The multiplication is done in `usize`: `area.width` is a `u16`, and a
    // terminal past 1023 columns would overflow one before the division.
    let inner = (usize::from(area.width) * 64 / 100)
        .saturating_sub(2)
        .max(1);
    let wrapped = u16::try_from(message.chars().count().div_ceil(inner)).unwrap_or(u16::MAX);
    // Two borders, a blank line and the closing hint sit around the message.
    wrapped.saturating_add(4).clamp(7, area.height)
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .split(area)[0];
    Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical)[0]
}

fn auth_environment_is_set() -> bool {
    env::vars_os().any(|(name, _)| {
        let name = name.to_string_lossy();
        name.ends_with("_API_KEY") || matches!(name.as_ref(), "ANTHROPIC_AUTH_TOKEN" | "HF_TOKEN")
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A terminal wide enough to overflow the `u16` the width is measured in
    /// used to size the notice from a wrapped-around number.
    #[test]
    fn sizes_a_notice_on_a_terminal_too_wide_for_a_u16_multiplication() {
        let narrow = Rect::new(0, 0, 100, 40);
        let wide = Rect::new(0, 0, 2000, 40);
        let message = "x".repeat(400);

        assert!(notice_height(&message, narrow) > notice_height(&message, wide));
        assert_eq!(notice_height(&message, wide), 7);
    }

    /// The footer centres each row inside a border, so a row wider than its
    /// box is silently clipped rather than wrapped. This pins the width that
    /// decides between one row and two to what the rows actually measure.
    #[test]
    fn the_launcher_filters_by_label_and_key_without_caring_about_case() {
        let installed = [
            Tool::Claude,
            Tool::by_key("gemini").unwrap(),
            Tool::by_key("kiro-cli").unwrap(),
        ];
        assert_eq!(launchable(&installed, ""), installed.to_vec());
        assert_eq!(launchable(&installed, "GEM"), vec![installed[1]]);
        assert_eq!(launchable(&installed, "kiro"), vec![installed[2]]);
        assert_eq!(
            launchable(&installed, "cli"),
            vec![installed[1], installed[2]]
        );
        assert!(launchable(&installed, "zzz").is_empty());
    }

    #[test]
    fn footer_rows_fit_the_widths_that_select_them() {
        // Wide mode shows the tools and the actions as one row each, so the
        // width that selects it is whichever of the two rows is longer.
        let wide = [
            shortcut_line(&WIDE_SHORTCUT_ROW),
            shortcut_line(&TOOL_SHORTCUTS),
        ]
        .iter()
        .map(|row| row.width() as u16 + 2)
        .max()
        .unwrap();
        assert!(wide <= WIDE_FOOTER_WIDTH, "{wide} exceeds the wide footer");
        assert!(
            wide > WIDE_FOOTER_WIDTH - 1,
            "the wide footer is {} wider than it needs to be, so terminals \
             that could show one row are given two",
            WIDE_FOOTER_WIDTH - wide
        );

        for row in NARROW_SHORTCUT_ROWS.iter().chain(NARROW_TOOL_ROWS.iter()) {
            let width = shortcut_line(row).width() as u16 + 2;
            assert!(width <= MINIMUM_WIDTH, "{width} exceeds the narrow footer");
        }

        let auth = shortcut_line(&AUTH_SHORTCUTS).width() as u16 + 2;
        let auth_popup = MINIMUM_WIDTH * 90 / 100;
        assert!(auth <= auth_popup, "{auth} exceeds the sign-in dialog");
    }

    #[test]
    fn abbreviates_paths_inside_the_home_directory() {
        let separator = std::path::MAIN_SEPARATOR;
        let home = PathBuf::from("/Users/example");
        let inside = home
            .join(".ditto")
            .join("profiles")
            .join("work")
            .join("claude");

        assert_eq!(
            shorten_home(&inside, &home),
            format!("~{separator}.ditto{separator}profiles{separator}work{separator}claude")
        );
        assert_eq!(
            shorten_home(Path::new("/opt/shared/claude"), &home),
            "/opt/shared/claude"
        );
    }

    #[test]
    fn keeps_the_tail_of_a_path_that_does_not_fit() {
        assert_eq!(
            truncate_start("~/.ditto/work/claude", 40),
            "~/.ditto/work/claude"
        );
        assert_eq!(truncate_start("~/.ditto/work/claude", 12), "…work/claude");
        assert_eq!(truncate_start("abc", 1), "…");
        assert_eq!(truncate_start("abc", 0), "");
        // Multi-byte characters must not be split mid-character.
        assert_eq!(truncate_start("→→→→", 3), "…→→");
    }

    #[test]
    fn reports_probes_as_pending_until_every_tool_answers() {
        let mut auth = ProfileAuth::default();
        assert!(auth.pending());

        auth.set(Tool::Claude, AuthStatus::SignedIn);
        auth.set(Tool::Codex, AuthStatus::SignedOut);
        assert!(auth.pending());

        auth.set(Tool::Fx, AuthStatus::SignedIn);
        assert!(auth.pending());

        auth.set(Tool::Opencode, AuthStatus::SignedIn);
        // OMP reports like the rest, so it holds the spinner open until it
        // answers.
        assert!(auth.pending());

        auth.set(Tool::Omp, AuthStatus::SignedOut);
        assert!(auth.pending());

        auth.set(Tool::PrimeAgent, AuthStatus::SignedIn);
        assert!(auth.pending());

        auth.set(Tool::Pi, AuthStatus::SignedOut);
        // The table's tools answer too, instantly, but they are answers all
        // the same.
        assert!(auth.pending());
        for tool in Tool::ALL {
            if matches!(tool, Tool::Generic(_)) {
                auth.set(tool, AuthStatus::Unavailable);
            }
        }
        assert!(!auth.pending());
        assert_eq!(auth.get(Tool::Opencode), Some(AuthStatus::SignedIn));
        assert_eq!(auth.get(Tool::Omp), Some(AuthStatus::SignedOut));
        assert_eq!(auth.get(Tool::PrimeAgent), Some(AuthStatus::SignedIn));
        assert_eq!(auth.get(Tool::Pi), Some(AuthStatus::SignedOut));
    }
}
