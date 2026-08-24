use iced::Color;
use iced::widget::{canvas, image, svg};
use iced::{ContentFit, Element, Length, Point, Rectangle, Renderer, Theme, mouse};
use std::path::Path;

/// The coordinate space every glyph below is drawn in; the canvas is scaled to whatever size
/// it is asked for.
const SIZE: f32 = 16.0;
const STROKE: f32 = 1.4;

const ART_SIZE: f32 = 15.0;

/// vscode-icons artwork, MIT, pinned at v12.13.0.
const FILE: &[u8] = include_bytes!("../../assets/icons/file.svg");
const FOLDER: &[u8] = include_bytes!("../../assets/icons/folder.svg");
const FOLDER_OPEN: &[u8] = include_bytes!("../../assets/icons/folder-open.svg");
const RUST: &[u8] = include_bytes!("../../assets/icons/rust.svg");
const CARGO: &[u8] = include_bytes!("../../assets/icons/cargo.svg");
const NIX: &[u8] = include_bytes!("../../assets/icons/nix.svg");
const TYPESCRIPT: &[u8] = include_bytes!("../../assets/icons/typescript.svg");
const TSX: &[u8] = include_bytes!("../../assets/icons/tsx.svg");
const JAVASCRIPT: &[u8] = include_bytes!("../../assets/icons/javascript.svg");
const JSON: &[u8] = include_bytes!("../../assets/icons/json.svg");
const NPM: &[u8] = include_bytes!("../../assets/icons/npm.svg");
const TOML: &[u8] = include_bytes!("../../assets/icons/toml.svg");
const YAML: &[u8] = include_bytes!("../../assets/icons/yaml.svg");
const MARKDOWN: &[u8] = include_bytes!("../../assets/icons/markdown.svg");
const SHELL: &[u8] = include_bytes!("../../assets/icons/shell.svg");
const GIT: &[u8] = include_bytes!("../../assets/icons/git.svg");
const TEXT: &[u8] = include_bytes!("../../assets/icons/text.svg");
const SVG: &[u8] = include_bytes!("../../assets/icons/svg.svg");
const IMAGE: &[u8] = include_bytes!("../../assets/icons/image.svg");
const HTML: &[u8] = include_bytes!("../../assets/icons/html.svg");
const CSS: &[u8] = include_bytes!("../../assets/icons/css.svg");

/// simple-icons artwork, CC0, pinned at v13. Single paths with no colour of their own, so
/// each is painted in whatever colour it is asked for.
const GITHUB: &[u8] = include_bytes!("../../assets/icons/github.svg");
const GITLAB: &[u8] = include_bytes!("../../assets/icons/gitlab.svg");
const GITEA: &[u8] = include_bytes!("../../assets/icons/gitea.svg");
const CODEBERG: &[u8] = include_bytes!("../../assets/icons/codeberg.svg");
const BITBUCKET: &[u8] = include_bytes!("../../assets/icons/bitbucket.svg");

/// A host nobody recognises gets the globe, which says "somewhere else" and nothing more.
pub fn forge<Message: 'static>(host: &str, size: f32) -> Element<'static, Message> {
    let host = host.to_ascii_lowercase();
    let logo = if host.contains("github") {
        GITHUB
    } else if host.contains("gitlab") {
        GITLAB
    } else if host.contains("codeberg") {
        CODEBERG
    } else if host.contains("gitea") || host.contains("forgejo") {
        GITEA
    } else if host.contains("bitbucket") {
        BITBUCKET
    } else {
        return sized(Glyph::Remote, size);
    };

    svg(svg::Handle::from_memory(logo))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.extended_palette().background.base.text),
        })
        .into()
}

pub fn file<Message: 'static>(path: &str) -> Element<'static, Message> {
    art(artwork(
        path.rsplit_once('/').map_or(path, |(_, name)| name),
    ))
}

pub fn folder<Message: 'static>(open: bool) -> Element<'static, Message> {
    art(if open { FOLDER_OPEN } else { FOLDER })
}

fn art<Message: 'static>(bytes: &'static [u8]) -> Element<'static, Message> {
    svg(svg::Handle::from_memory(bytes))
        .width(Length::Fixed(ART_SIZE))
        .height(Length::Fixed(ART_SIZE))
        .into()
}

/// A name that carries no known type gets the generic page: the nearest plausible logo would
/// be a worse answer, because the reader believes it.
fn artwork(name: &str) -> &'static [u8] {
    match name {
        "Cargo.toml" | "Cargo.lock" => CARGO,
        "package.json" | "package-lock.json" => NPM,
        ".gitignore" | ".gitattributes" => GIT,
        _ => match name.rsplit_once('.') {
            Some((_, "rs")) => RUST,
            Some((_, "nix")) => NIX,
            Some((_, "ts")) => TYPESCRIPT,
            Some((_, "tsx")) => TSX,
            Some((_, "js" | "mjs" | "cjs")) => JAVASCRIPT,
            Some((_, "json")) => JSON,
            Some((_, "toml")) => TOML,
            Some((_, "yaml" | "yml")) => YAML,
            Some((_, "md")) => MARKDOWN,
            Some((_, "sh" | "bash" | "zsh")) => SHELL,
            Some((_, "txt")) => TEXT,
            Some((_, "svg")) => SVG,
            Some((_, "png" | "jpg" | "jpeg" | "webp" | "gif")) => IMAGE,
            Some((_, "html")) => HTML,
            Some((_, "css")) => CSS,
            Some(_) | None => FILE,
        },
    }
}

/// `file` is the icon inside the repository the state file remembers; without one the
/// repository gets the neutral glyph rather than a guess.
pub fn repository<Message: 'static>(file: Option<&Path>, size: f32) -> Element<'static, Message> {
    let fixed = Length::Fixed(size);

    match file {
        None => sized(Glyph::Repository, size),
        Some(file) if file.extension().is_some_and(|kind| kind == "svg") => {
            svg(svg::Handle::from_path(file))
                .width(fixed)
                .height(fixed)
                .into()
        }
        Some(file) => image(image::Handle::from_path(file))
            .content_fit(ContentFit::Contain)
            .width(fixed)
            .height(fixed)
            .into(),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Glyph {
    Sliders,
    Bell,
    Cross,
    /// The neutral card, for a repository with no icon of its own.
    Repository,
    Branch,
    Tag,
    Remote,
    Head,
    Stash,
    Plus,
    Unfolded,
    Folded,
    Back,
    Worktree,
    PullRequest,
    Fetch,
    Push,
}

struct Icon {
    glyph: Glyph,
    colour: Option<Color>,
}

pub fn icon<Message: 'static>(glyph: Glyph) -> Element<'static, Message> {
    sized(glyph, SIZE)
}

pub fn sized<Message: 'static>(glyph: Glyph, size: f32) -> Element<'static, Message> {
    canvas(Icon {
        glyph,
        colour: None,
    })
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .into()
}

/// In a colour of its own rather than the one the theme reads text in.
pub fn coloured<Message: 'static>(
    glyph: Glyph,
    size: f32,
    colour: Color,
) -> Element<'static, Message> {
    canvas(Icon {
        glyph,
        colour: Some(colour),
    })
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .into()
}

impl<Message> canvas::Program<Message> for Icon {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let colour = self
            .colour
            .unwrap_or_else(|| theme.extended_palette().background.base.text);
        let stroke = canvas::Stroke::default()
            .with_color(colour)
            .with_width(STROKE);
        frame.scale(bounds.width / SIZE);

        match self.glyph {
            Glyph::Sliders => {
                for (index, y) in [3.5_f32, 8.0, 12.5].into_iter().enumerate() {
                    frame.stroke(
                        &canvas::Path::line(Point::new(1.5, y), Point::new(14.5, y)),
                        stroke,
                    );
                    let knob = [5.0, 10.0, 6.5][index];
                    frame.fill(&canvas::Path::circle(Point::new(knob, y), 2.0), colour);
                }
            }
            Glyph::Bell => {
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(3.5, 11.0));
                        builder.line_to(Point::new(3.5, 7.5));
                        builder.quadratic_curve_to(Point::new(3.5, 3.0), Point::new(8.0, 2.5));
                        builder.quadratic_curve_to(Point::new(12.5, 3.0), Point::new(12.5, 7.5));
                        builder.line_to(Point::new(12.5, 11.0));
                    }),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(2.0, 11.0), Point::new(14.0, 11.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(6.5, 13.0));
                        builder.quadratic_curve_to(Point::new(8.0, 14.5), Point::new(9.5, 13.0));
                    }),
                    stroke,
                );
            }
            Glyph::Unfolded | Glyph::Folded => {
                let right = matches!(self.glyph, Glyph::Folded);
                let points = if right {
                    [(5.5, 3.5), (11.5, 8.0), (5.5, 12.5)]
                } else {
                    [(3.5, 5.5), (8.0, 11.5), (12.5, 5.5)]
                };

                frame.fill(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(points[0].0, points[0].1));
                        builder.line_to(Point::new(points[1].0, points[1].1));
                        builder.line_to(Point::new(points[2].0, points[2].1));
                        builder.close();
                    }),
                    colour,
                );
            }
            Glyph::Back => {
                frame.stroke(
                    &canvas::Path::line(Point::new(3.0, 8.0), Point::new(13.0, 8.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(7.0, 4.0));
                        builder.line_to(Point::new(3.0, 8.0));
                        builder.line_to(Point::new(7.0, 12.0));
                    }),
                    stroke,
                );
            }
            Glyph::Worktree => {
                frame.stroke(
                    &canvas::Path::rectangle(Point::new(2.0, 5.0), iced::Size::new(12.0, 9.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(2.0, 5.0));
                        builder.line_to(Point::new(2.0, 2.5));
                        builder.line_to(Point::new(7.0, 2.5));
                        builder.line_to(Point::new(8.5, 5.0));
                    }),
                    stroke,
                );
            }
            Glyph::PullRequest => {
                frame.stroke(
                    &canvas::Path::line(Point::new(4.0, 4.5), Point::new(4.0, 11.5)),
                    stroke,
                );
                frame.fill(&canvas::Path::circle(Point::new(4.0, 12.5), 1.6), colour);
                frame.fill(&canvas::Path::circle(Point::new(11.5, 3.5), 1.6), colour);
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(11.5, 5.5));
                        builder.line_to(Point::new(11.5, 9.0));
                    }),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(9.5, 9.0));
                        builder.line_to(Point::new(11.5, 12.0));
                        builder.line_to(Point::new(13.5, 9.0));
                    }),
                    stroke,
                );
            }
            Glyph::Fetch | Glyph::Push => {
                // The same drawing either way up.
                let down = matches!(self.glyph, Glyph::Fetch);
                let (from, to) = if down { (3.0, 9.5) } else { (9.5, 3.0) };

                frame.stroke(
                    &canvas::Path::line(Point::new(8.0, from), Point::new(8.0, to)),
                    stroke,
                );
                for side in [-1.0_f32, 1.0] {
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(8.0 + side * 3.0, to + if down { -3.0 } else { 3.0 }),
                            Point::new(8.0, to),
                        ),
                        stroke,
                    );
                }
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(2.5, 10.0));
                        builder.line_to(Point::new(2.5, 13.5));
                        builder.line_to(Point::new(13.5, 13.5));
                        builder.line_to(Point::new(13.5, 10.0));
                    }),
                    stroke,
                );
            }
            Glyph::Plus => {
                frame.stroke(
                    &canvas::Path::line(Point::new(8.0, 3.5), Point::new(8.0, 12.5)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(3.5, 8.0), Point::new(12.5, 8.0)),
                    stroke,
                );
            }
            Glyph::Cross => {
                frame.stroke(
                    &canvas::Path::line(Point::new(4.0, 4.0), Point::new(12.0, 12.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(12.0, 4.0), Point::new(4.0, 12.0)),
                    stroke,
                );
            }
            Glyph::Repository => {
                frame.stroke(
                    &canvas::Path::rounded_rectangle(
                        Point::new(2.0, 2.0),
                        iced::Size::new(12.0, 12.0),
                        2.0.into(),
                    ),
                    stroke,
                );
                frame.fill(&canvas::Path::circle(Point::new(8.0, 8.0), 2.0), colour);
            }
            Glyph::Branch => {
                frame.stroke(
                    &canvas::Path::line(Point::new(4.5, 4.0), Point::new(4.5, 12.5)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(11.5, 5.0));
                        builder.line_to(Point::new(11.5, 7.0));
                        builder.quadratic_curve_to(Point::new(11.5, 9.5), Point::new(8.0, 9.8));
                        builder.quadratic_curve_to(Point::new(4.5, 10.1), Point::new(4.5, 12.0));
                    }),
                    stroke,
                );
                frame.fill(&canvas::Path::circle(Point::new(4.5, 3.0), 1.9), colour);
                frame.fill(&canvas::Path::circle(Point::new(11.5, 3.5), 1.9), colour);
                frame.fill(&canvas::Path::circle(Point::new(4.5, 13.0), 1.9), colour);
            }
            Glyph::Tag => {
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(2.0, 8.6));
                        builder.line_to(Point::new(8.6, 2.0));
                        builder.line_to(Point::new(14.0, 2.0));
                        builder.line_to(Point::new(14.0, 7.4));
                        builder.line_to(Point::new(7.4, 14.0));
                        builder.close();
                    }),
                    stroke,
                );
                frame.fill(&canvas::Path::circle(Point::new(11.2, 4.8), 1.4), colour);
            }
            Glyph::Remote => {
                frame.stroke(&canvas::Path::circle(Point::new(8.0, 8.0), 6.0), stroke);
                frame.stroke(
                    &canvas::Path::line(Point::new(2.0, 8.0), Point::new(14.0, 8.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::new(|builder| {
                        builder.move_to(Point::new(8.0, 2.0));
                        builder.quadratic_curve_to(Point::new(4.4, 8.0), Point::new(8.0, 14.0));
                        builder.quadratic_curve_to(Point::new(11.6, 8.0), Point::new(8.0, 2.0));
                    }),
                    stroke,
                );
            }
            Glyph::Head => {
                frame.stroke(&canvas::Path::circle(Point::new(8.0, 8.0), 5.6), stroke);
                frame.fill(&canvas::Path::circle(Point::new(8.0, 8.0), 2.6), colour);
            }
            Glyph::Stash => {
                frame.stroke(
                    &canvas::Path::rounded_rectangle(
                        Point::new(2.0, 4.0),
                        iced::Size::new(12.0, 9.0),
                        1.5.into(),
                    ),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(2.0, 9.0), Point::new(5.5, 9.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(10.5, 9.0), Point::new(14.0, 9.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(5.5, 9.0), Point::new(6.5, 11.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(10.5, 9.0), Point::new(9.5, 11.0)),
                    stroke,
                );
                frame.stroke(
                    &canvas::Path::line(Point::new(6.5, 11.0), Point::new(9.5, 11.0)),
                    stroke,
                );
            }
        }

        vec![frame.into_geometry()]
    }
}
