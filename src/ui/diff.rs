use super::Message;
use iced::alignment::Horizontal;
use iced::widget::{Column, Space, canvas, container, rich_text, row, scrollable, text};
use iced::{Color, Element, Fill, Font, Length, Point, Rectangle, Renderer, Size, Theme, mouse};
use std::path::Path;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

const SIZE: f32 = super::BODY;
/// Wide enough for five digits at [`SIZE`], which the gutters share with the code so a
/// number and its line sit on one baseline.
const GUTTER_WIDTH: f32 = 40.0;
const EDGE_WIDTH: f32 = 2.0;
const OVERVIEW_WIDTH: f32 = 12.0;
const MARK_HEIGHT: f32 = 2.0;
const MARK_INSET: f32 = 2.0;
/// Kept above the first change when a file opens on it.
const CONTEXT_LINES: usize = 3;

/// Every row is laid out eagerly, and the highlighter walks the file line by line, so past
/// a few thousand lines both cost more than the colour is worth.
const HIGHLIGHT_LIMIT: usize = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Highlighted,
    Raw,
}

pub struct Body {
    raw: String,
    note: Option<String>,
    rows: Vec<Line>,
}

struct Line {
    old: Option<usize>,
    new: Option<usize>,
    tag: Tag,
    pieces: Vec<Piece>,
}

struct Piece {
    text: String,
    colour: Option<Color>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Unchanged,
    Added,
    Removed,
}

/// `raw` is the tight-context diff shown by [`Mode::Raw`]; `whole_file` is the same diff
/// taken with the whole file as context, which is what carries the tags for every line.
pub fn prepare(raw: String, whole_file: &str, path: &str, dark: bool) -> Body {
    if whole_file
        .lines()
        .any(|line| line.starts_with("Binary files") || line.starts_with("GIT binary patch"))
    {
        return Body {
            raw,
            note: Some("this file is binary, so there is nothing to show".to_owned()),
            rows: Vec::new(),
        };
    }

    let tagged = tag_lines(whole_file);

    if tagged.len() > HIGHLIGHT_LIMIT {
        return Body {
            raw,
            note: Some(format!(
                "{} lines is more than gg highlights, so this is shown plain",
                tagged.len()
            )),
            rows: plain_rows(tagged),
        };
    }

    match highlight(&tagged, path, dark) {
        Ok(coloured) => Body {
            raw,
            note: None,
            rows: tagged
                .into_iter()
                .zip(coloured)
                .map(|((old, new, tag, _), pieces)| Line {
                    old,
                    new,
                    tag,
                    pieces,
                })
                .collect(),
        },
        Err(error) => Body {
            raw,
            note: Some(format!("could not highlight this file: {error}")),
            rows: plain_rows(tagged),
        },
    }
}

type Tagged<'a> = (Option<usize>, Option<usize>, Tag, &'a str);

fn tag_lines(whole_file: &str) -> Vec<Tagged<'_>> {
    let mut rows = Vec::new();
    let mut old = 0;
    let mut new = 0;
    let mut in_hunk = false;

    for line in whole_file.lines() {
        if let Some(start) = hunk_start(line) {
            (old, new) = start;
            in_hunk = true;
            continue;
        }
        if !in_hunk || line.starts_with('\\') {
            continue;
        }

        let (tag, text) = match line.as_bytes().first() {
            Some(b'+') => (Tag::Added, &line[1..]),
            Some(b'-') => (Tag::Removed, &line[1..]),
            Some(b' ') => (Tag::Unchanged, &line[1..]),
            None => (Tag::Unchanged, line),
            Some(_) => continue,
        };

        let numbers = match tag {
            Tag::Added => {
                new += 1;
                (None, Some(new))
            }
            Tag::Removed => {
                old += 1;
                (Some(old), None)
            }
            Tag::Unchanged => {
                old += 1;
                new += 1;
                (Some(old), Some(new))
            }
        };

        rows.push((numbers.0, numbers.1, tag, text));
    }

    rows
}

/// The line before the first one the hunk covers, so a row can count up before it is used.
fn hunk_start(line: &str) -> Option<(usize, usize)> {
    let mut fields = line.strip_prefix("@@ -")?.split(' ');
    let old = fields.next()?;
    let new = fields.next()?.strip_prefix('+')?;

    let number = |field: &str| {
        field
            .split(',')
            .next()
            .and_then(|count| count.parse::<usize>().ok())
            .map(|count| count.saturating_sub(1))
    };

    Some((number(old)?, number(new)?))
}

fn plain_rows(tagged: Vec<Tagged<'_>>) -> Vec<Line> {
    tagged
        .into_iter()
        .map(|(old, new, tag, text)| Line {
            old,
            new,
            tag,
            pieces: vec![Piece {
                text: text.to_owned(),
                colour: None,
            }],
        })
        .collect()
}

fn highlight(
    tagged: &[Tagged<'_>],
    path: &str,
    dark: bool,
) -> Result<Vec<Vec<Piece>>, syntect::Error> {
    let syntaxes = syntaxes();
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let syntax = syntaxes
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());

    let mut highlighter = HighlightLines::new(syntax, syntect_theme(dark));
    let mut rows = Vec::with_capacity(tagged.len());

    for (_, _, _, text) in tagged {
        // The syntax set carries the newline in its patterns, so a line without one can
        // leave the parser stuck mid-context.
        let line = format!("{text}\n");
        let mut pieces: Vec<Piece> = Vec::new();

        for (style, fragment) in highlighter.highlight_line(&line, syntaxes)? {
            let fragment = fragment.trim_end_matches('\n');
            if fragment.is_empty() {
                continue;
            }

            let foreground = style.foreground;
            let colour = Color::from_rgb8(foreground.r, foreground.g, foreground.b);
            match pieces.last_mut() {
                Some(last) if last.colour == Some(colour) => last.text.push_str(fragment),
                _ => pieces.push(Piece {
                    text: fragment.to_owned(),
                    colour: Some(colour),
                }),
            }
        }

        rows.push(pieces);
    }

    Ok(rows)
}

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

fn syntect_theme(dark: bool) -> &'static syntect::highlighting::Theme {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);

    &themes.themes[if dark {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    }]
}

/// Drawn beside the scrollbar, in the same shape as the file.
struct Overview<'a> {
    rows: &'a [Line],
    cache: &'a canvas::Cache,
}

impl canvas::Program<Message> for Overview<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            if self.rows.is_empty() {
                return;
            }

            // A change of one line in a file of ten thousand would be half a pixel tall
            // and invisible, so a mark is never thinner than this.
            let step = bounds.height / self.rows.len() as f32;
            let height = step.max(MARK_HEIGHT);

            for (index, line) in self.rows.iter().enumerate() {
                let Some(colour) = hue(theme, line.tag) else {
                    continue;
                };

                frame.fill_rectangle(
                    Point::new(MARK_INSET, index as f32 * step),
                    Size::new(bounds.width - MARK_INSET * 2.0, height),
                    colour,
                );
            }
        });

        vec![geometry]
    }
}

/// How far down the file the first change is, as a share of it.
pub fn first_change(body: &Body) -> Option<f32> {
    let total = body.rows.len();
    let first = body
        .rows
        .iter()
        .position(|line| line.tag != Tag::Unchanged)?;

    // Landed a little above, so the change opens with the lines that lead into it.
    Some(((first.saturating_sub(CONTEXT_LINES)) as f32 / total as f32).clamp(0.0, 1.0))
}

pub fn view<'a>(
    body: &'a Body,
    mode: Mode,
    scroll: iced::widget::Id,
    cache: &'a canvas::Cache,
) -> Element<'a, Message> {
    let content = match mode {
        Mode::Raw => raw(&body.raw),
        Mode::Highlighted => highlighted(body),
    };

    let pane = scrollable(content).id(scroll).width(Fill).height(Fill);
    if body.rows.is_empty() {
        return pane.into();
    }

    row![
        pane,
        canvas(Overview {
            rows: &body.rows,
            cache,
        })
        .width(Length::Fixed(OVERVIEW_WIDTH))
        .height(Fill),
    ]
    .into()
}

fn highlighted(body: &Body) -> Element<'_, Message> {
    if body.rows.is_empty() && body.note.is_none() {
        return note("no textual difference");
    }

    let mut rows: Vec<Element<'_, Message>> = Vec::with_capacity(body.rows.len() + 1);
    if let Some(text) = &body.note {
        rows.push(note(text));
    }
    rows.extend(body.rows.iter().map(line));

    Column::with_children(rows).into()
}

fn line(line: &Line) -> Element<'_, Message> {
    let tag = line.tag;
    let spans: Vec<text::Span<'_, ()>> = line
        .pieces
        .iter()
        .map(|piece| text::Span::new(piece.text.as_str()).color_maybe(piece.colour))
        .collect();

    // The pane does not scroll sideways, so a long line has to wrap or it cannot be read.
    let code = rich_text(spans).size(SIZE).font(Font::MONOSPACE);

    let row = row![
        container(Space::new().width(EDGE_WIDTH).height(Fill)).style(move |theme: &Theme| {
            container::Style {
                background: hue(theme, tag).map(Into::into),
                ..container::Style::default()
            }
        }),
        gutter(line.old),
        gutter(line.new),
        code,
    ]
    .spacing(6);

    container(row)
        .width(Fill)
        .style(move |theme: &Theme| container::Style {
            background: hue(theme, tag)
                .map(|colour| Color { a: 0.14, ..colour })
                .map(Into::into),
            ..container::Style::default()
        })
        .into()
}

fn gutter(number: Option<usize>) -> Element<'static, Message> {
    text(number.map(|number| number.to_string()).unwrap_or_default())
        .size(SIZE)
        .font(Font::MONOSPACE)
        .style(text::secondary)
        .width(Length::Fixed(GUTTER_WIDTH))
        .align_x(Horizontal::Right)
        .into()
}

fn hue(theme: &Theme, tag: Tag) -> Option<Color> {
    match tag {
        Tag::Unchanged => None,
        Tag::Added => Some(theme.palette().success),
        Tag::Removed => Some(theme.palette().danger),
    }
}

fn note(line: &str) -> Element<'_, Message> {
    text(line).size(12).style(text::secondary).into()
}

fn raw(body: &str) -> Element<'_, Message> {
    if body.trim().is_empty() {
        return note("no textual difference");
    }

    let lines = body.lines().map(|line| {
        let style: fn(&iced::Theme) -> text::Style = if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
        {
            text::secondary
        } else if line.starts_with("@@") {
            text::primary
        } else if line.starts_with('+') {
            text::success
        } else if line.starts_with('-') {
            text::danger
        } else {
            text::default
        };

        text(line)
            .size(SIZE)
            .font(Font::MONOSPACE)
            .style(style)
            .into()
    });

    Column::with_children(lines).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHOLE_FILE: &str = concat!(
        "diff --git a/a.rs b/a.rs\n",
        "index 1111111..2222222 100644\n",
        "--- a/a.rs\n",
        "+++ b/a.rs\n",
        "@@ -1,3 +1,3 @@\n",
        " fn main() {}\n",
        "-let old = 1;\n",
        "+let new = 1;\n",
    );

    #[test]
    fn a_binary_file_says_so_and_shows_no_rows() {
        let body = prepare(
            String::new(),
            "diff --git a/logo.png b/logo.png\nBinary files a/logo.png and b/logo.png differ\n",
            "logo.png",
            true,
        );

        assert!(body.rows.is_empty());
        assert!(body.note.is_some_and(|note| note.contains("binary")));
    }

    #[test]
    fn a_file_with_no_change_has_nothing_to_show() {
        let body = prepare(String::new(), "", "a.rs", true);

        assert!(body.rows.is_empty());
        assert!(body.note.is_none());
    }

    #[test]
    fn every_line_keeps_its_tag_and_its_numbers() {
        let body = prepare(String::new(), WHOLE_FILE, "a.rs", true);

        let rows: Vec<_> = body
            .rows
            .iter()
            .map(|line| (line.old, line.new, line.tag))
            .collect();

        assert_eq!(
            rows,
            vec![
                (Some(1), Some(1), Tag::Unchanged),
                (Some(2), None, Tag::Removed),
                (None, Some(2), Tag::Added),
            ]
        );
        assert_eq!(
            body.rows[2]
                .pieces
                .iter()
                .map(|piece| piece.text.as_str())
                .collect::<String>(),
            "let new = 1;"
        );
    }
}
