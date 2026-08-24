use iced::theme::{Custom, Palette};
use iced::{Color, Theme};
use std::sync::Arc;
use zbus::zvariant::{OwnedValue, Value};

/// Read through the XDG desktop portal rather than the desktop's own configuration files,
/// so this behaves the same wherever the portal is implemented.
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Choice {
    System,
    Dark,
    Light,
    Dracula,
    TokyoNight,
    Nord,
    GruvboxDark,
    CatppuccinMocha,
    SolarizedLight,
}

impl Choice {
    pub const ALL: [Self; 9] = [
        Self::System,
        Self::Dark,
        Self::Light,
        Self::Dracula,
        Self::TokyoNight,
        Self::Nord,
        Self::GruvboxDark,
        Self::CatppuccinMocha,
        Self::SolarizedLight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Dracula => "dracula",
            Self::TokyoNight => "tokyo night",
            Self::Nord => "nord",
            Self::GruvboxDark => "gruvbox dark",
            Self::CatppuccinMocha => "catppuccin mocha",
            Self::SolarizedLight => "solarized light",
        }
    }
}

/// Colours by what each one means rather than by what it looks like. Every hue on screen
/// comes from here, so a theme is one table and nothing else.
#[derive(Debug, Clone, Copy)]
pub struct Colours {
    pub background: Color,
    pub text: Color,
    /// Names that are present but not the point: a directory, a column heading.
    pub text_secondary: Color,
    /// Counts and notes beside something else.
    pub text_faint: Color,
    /// The accent: what is focused, selected, or being pointed at.
    pub focus: Color,
    pub added: Color,
    pub deleted: Color,
    pub modified: Color,
    pub renamed: Color,
    /// Tree connectors and anything else drawn to be followed rather than read.
    pub guide: Color,
    /// The panes either side of the history: the sidebar, the detail, the settings page.
    pub pane: Color,
    /// Chrome that sits above the panes rather than beside them: the bar naming the open
    /// repository.
    pub chrome: Color,
    /// Behind a face in the graph, so a pale mark still reads against it.
    pub inset: Color,
    /// One per lane, repeating. A lane keeps its colour for as long as it is open.
    pub lanes: [Color; 7],
    pub dark: bool,
}

pub fn resolve(choice: Choice) -> Theme {
    let colours = colours(choice);
    let palette = Palette {
        background: colours.background,
        text: colours.text,
        primary: colours.focus,
        success: colours.added,
        warning: colours.lanes[2],
        danger: colours.deleted,
    };

    // iced derives its eight shades of the background by nudging it towards the text. A
    // base16 theme already says what those shades are, so the ones the interface leans on
    // are replaced with the theme's own.
    Theme::Custom(Arc::new(Custom::with_fn(
        choice.label().to_owned(),
        palette,
        move |palette| {
            let mut extended = iced::theme::palette::Extended::generate(palette);
            let pair = |colour| iced::theme::palette::Pair::new(colour, colours.text);

            extended.background.weakest = pair(colours.background);
            extended.background.weaker = pair(colours.pane);
            extended.background.weak = pair(colours.pane);
            extended.background.neutral = pair(colours.chrome);
            extended.background.strong = pair(colours.chrome);
            extended.background.stronger = pair(colours.guide);
            extended
        },
    )))
}

pub fn colours(choice: Choice) -> Colours {
    match choice {
        // The portal says only whether the desktop is light or dark and which accent it
        // uses; the rest of the palette is ours either way.
        Choice::System => {
            let mut colours = if system_prefers_light() {
                from(LIGHT)
            } else {
                from(DARK)
            };
            if let Some(accent) = accent_colour() {
                colours.focus = accent;
            }
            colours
        }
        Choice::Dark => from(DARK),
        Choice::Light => from(LIGHT),
        Choice::Dracula => from(DRACULA),
        Choice::TokyoNight => from(TOKYO_NIGHT),
        Choice::Nord => from(NORD),
        Choice::GruvboxDark => from(GRUVBOX_DARK),
        Choice::CatppuccinMocha => from(CATPPUCCIN_MOCHA),
        Choice::SolarizedLight => from(SOLARIZED_LIGHT),
    }
}

/// The sixteen colours a base16 scheme is published as.
///
/// 00 background, 01 raised, 02 selection, 03 comment, 04 dim text, 05 text, 06 and 07
/// brighter still, then 08 red, 09 orange, 0A yellow, 0B green, 0C cyan, 0D blue,
/// 0E magenta, 0F brown.
struct Base16([Color; 16]);

fn from(base: Base16) -> Colours {
    let hue = base.0;

    Colours {
        background: hue[0],
        text: hue[5],
        text_secondary: hue[4],
        text_faint: hue[3],
        focus: hue[13],
        added: hue[11],
        deleted: hue[8],
        modified: hue[13],
        renamed: hue[14],
        guide: hue[2],
        pane: hue[1],
        chrome: hue[2],
        inset: hue[1],
        lanes: [hue[13], hue[11], hue[9], hue[14], hue[12], hue[8], hue[10]],
        dark: luminance(hue[0]) < luminance(hue[5]),
    }
}

/// Rough, and only ever used to ask which of two colours is the lighter one.
fn luminance(colour: Color) -> f32 {
    0.2126 * colour.r + 0.7152 * colour.g + 0.0722 * colour.b
}

fn system_prefers_light() -> bool {
    let Some(value) = portal("color-scheme") else {
        return false;
    };

    // The portal reports 0 for no preference, 1 for dark and 2 for light.
    matches!(&*value, Value::U32(2))
}

fn accent_colour() -> Option<Color> {
    let value = portal("accent-color")?;
    let Value::Structure(structure) = &*value else {
        return None;
    };

    let components: Vec<f64> = structure
        .fields()
        .iter()
        .filter_map(|field| match field {
            Value::F64(component) => Some(*component),
            _ => None,
        })
        .collect();

    match components.as_slice() {
        [red, green, blue] => Some(Color::from_rgb(*red as f32, *green as f32, *blue as f32)),
        _ => None,
    }
}

fn portal(key: &str) -> Option<OwnedValue> {
    let connection = zbus::blocking::Connection::session().ok()?;
    let reply = connection
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some(SETTINGS),
            "ReadOne",
            &(APPEARANCE, key),
        )
        .ok()?;

    reply.body().deserialize::<OwnedValue>().ok()
}

pub fn surface(theme: &Theme) -> iced::widget::container::Style {
    filled(theme.extended_palette().background.weak.color)(theme)
}

pub fn chrome(theme: &Theme) -> iced::widget::container::Style {
    filled(theme.extended_palette().background.strong.color)(theme)
}

/// A row or a control that says it takes a press by lighting up under the pointer. The one
/// style the toolbar, the context menus and the remote dialog share, so a pressable thing
/// looks the same wherever it is drawn.
pub fn row(theme: &Theme, status: iced::widget::button::Status) -> iced::widget::button::Style {
    let palette = theme.extended_palette();
    let lit = matches!(
        status,
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed
    );

    iced::widget::button::Style {
        background: lit.then(|| palette.background.strong.color.into()),
        text_color: palette.background.base.text,
        border: iced::Border {
            radius: 5.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

pub fn filled(colour: Color) -> impl Fn(&Theme) -> iced::widget::container::Style + Copy {
    move |_: &Theme| iced::widget::container::Style {
        background: Some(colour.into()),
        ..iced::widget::container::Style::default()
    }
}

const fn hex(value: u32) -> Color {
    Color::from_rgb8(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

const DARK: Base16 = Base16([
    hex(0x1b1e24),
    hex(0x22262e),
    hex(0x383d47),
    hex(0x71747d),
    hex(0xa2a6ad),
    hex(0xe0e2e6),
    hex(0xf0f1f3),
    hex(0xffffff),
    hex(0xf28680),
    hex(0xf2b06a),
    hex(0xd9cc7a),
    hex(0x5fbf87),
    hex(0x73d1d1),
    hex(0x86a8f7),
    hex(0xb79bf0),
    hex(0xd98cc0),
]);

const LIGHT: Base16 = Base16([
    hex(0xf7f7f8),
    hex(0xeceef1),
    hex(0xd7dae0),
    hex(0x8a8f99),
    hex(0x5b606b),
    hex(0x24272e),
    hex(0x14161a),
    hex(0x000000),
    hex(0xb43230),
    hex(0xa9601c),
    hex(0x8a6d13),
    hex(0x1c7340),
    hex(0x14707a),
    hex(0x2f5bd0),
    hex(0x7a3fbd),
    hex(0xa03a86),
]);

const DRACULA: Base16 = Base16([
    hex(0x282a36),
    hex(0x3a3c4e),
    hex(0x4d4f68),
    hex(0x626483),
    hex(0x62d6e8),
    hex(0xf8f8f2),
    hex(0xf1f2f8),
    hex(0xffffff),
    hex(0xff5555),
    hex(0xffb86c),
    hex(0xf1fa8c),
    hex(0x50fa7b),
    hex(0x8be9fd),
    hex(0xbd93f9),
    hex(0xff79c6),
    hex(0x00f769),
]);

const TOKYO_NIGHT: Base16 = Base16([
    hex(0x1a1b26),
    hex(0x24283b),
    hex(0x2f3549),
    hex(0x565f89),
    hex(0x9aa5ce),
    hex(0xc0caf5),
    hex(0xd5d6db),
    hex(0xffffff),
    hex(0xf7768e),
    hex(0xff9e64),
    hex(0xe0af68),
    hex(0x9ece6a),
    hex(0x7dcfff),
    hex(0x7aa2f7),
    hex(0xbb9af7),
    hex(0xc0a36e),
]);

const NORD: Base16 = Base16([
    hex(0x2e3440),
    hex(0x3b4252),
    hex(0x434c5e),
    hex(0x4c566a),
    hex(0xd8dee9),
    hex(0xe5e9f0),
    hex(0xeceff4),
    hex(0xffffff),
    hex(0xbf616a),
    hex(0xd08770),
    hex(0xebcb8b),
    hex(0xa3be8c),
    hex(0x88c0d0),
    hex(0x81a1c1),
    hex(0xb48ead),
    hex(0x5e81ac),
]);

const GRUVBOX_DARK: Base16 = Base16([
    hex(0x1d2021),
    hex(0x3c3836),
    hex(0x504945),
    hex(0x665c54),
    hex(0xbdae93),
    hex(0xd5c4a1),
    hex(0xebdbb2),
    hex(0xfbf1c7),
    hex(0xfb4934),
    hex(0xfe8019),
    hex(0xfabd2f),
    hex(0xb8bb26),
    hex(0x8ec07c),
    hex(0x83a598),
    hex(0xd3869b),
    hex(0xd65d0e),
]);

const CATPPUCCIN_MOCHA: Base16 = Base16([
    hex(0x1e1e2e),
    hex(0x181825),
    hex(0x313244),
    hex(0x585b70),
    hex(0xa6adc8),
    hex(0xcdd6f4),
    hex(0xf5e0dc),
    hex(0xb4befe),
    hex(0xf38ba8),
    hex(0xfab387),
    hex(0xf9e2af),
    hex(0xa6e3a1),
    hex(0x94e2d5),
    hex(0x89b4fa),
    hex(0xcba6f7),
    hex(0xf2cdcd),
]);

const SOLARIZED_LIGHT: Base16 = Base16([
    hex(0xfdf6e3),
    hex(0xeee8d5),
    hex(0x93a1a1),
    hex(0x839496),
    hex(0x657b83),
    hex(0x586e75),
    hex(0x073642),
    hex(0x002b36),
    hex(0xdc322f),
    hex(0xcb4b16),
    hex(0xb58900),
    hex(0x859900),
    hex(0x2aa198),
    hex(0x268bd2),
    hex(0x6c71c4),
    hex(0xd33682),
]);
