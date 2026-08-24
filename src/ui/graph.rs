use super::theme;
use crate::avatar;
use crate::git::history::GraphRow;
use gix::ObjectId;
use iced::widget::{canvas, image};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

pub const ROW_HEIGHT: f32 = 26.0;
pub const LANE_WIDTH: f32 = 22.0;
pub const GRAPH_INSET: f32 = 10.0;

const FACE_RADIUS: f32 = 9.5;
const RING_WIDTH: f32 = 1.6;
const CONNECTOR_WIDTH: f32 = 1.6;
/// Any more and the corners of two lanes crossing at once would run into each other.
const CORNER_RADIUS: f32 = 6.0;
/// What is not a commit is drawn broken: the working tree, and a stash.
const DASH: &[f32] = &[3.0, 3.0];
/// The generated mark's square as a share of the node diameter. Any wider and its corners
/// would leave the circle.
const MARK_SHARE: f32 = 0.66;

#[derive(Clone)]
pub struct Author {
    pub fingerprint: [u8; 32],
    pub generated: avatar::Generated,
}

const LINE_WIDTH: f32 = 1.6;

pub struct Graph<'a> {
    pub rows: &'a [GraphRow],
    pub cache: &'a canvas::Cache,
    /// Draws a dashed node above the newest commit.
    pub working_tree: bool,
    /// The top of each row, one per entry in `rows`. Not a fixed grid: the date separators
    /// take a slot of their own between commits.
    pub tops: &'a [f32],
    /// The author of each row, one per entry in `rows`.
    pub authors: &'a [Author],
    /// The pictures that have arrived so far, by author fingerprint.
    pub pictures: &'a HashMap<[u8; 32], super::Faces>,
    /// The commits a stash points at.
    pub stashes: &'a HashSet<ObjectId>,
    /// The commits carrying a branch, tag or head label.
    pub labelled: &'a HashSet<ObjectId>,
    /// The rows worth drawing. The canvas is as tall as the whole history, and drawing all
    /// of nixpkgs into it is felt on every scroll, resize and arriving face.
    pub range: Range<usize>,
    pub colours: theme::Colours,
}

impl<Message> canvas::Program<Message> for Graph<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let offset = usize::from(self.working_tree);

            if self.working_tree && self.range.start == 0 {
                self.draw_working_tree(frame);
            }

            let drawn = self.range.start..self.range.end.min(self.rows.len());
            for (index, row) in drawn.clone().zip(&self.rows[drawn]) {
                let top = self
                    .tops
                    .get(index)
                    .copied()
                    .unwrap_or((index + offset) as f32 * ROW_HEIGHT);
                // A separator leaves a slot with no row in it, so a line has to reach the
                // next commit rather than one row down.
                let bottom = self
                    .tops
                    .get(index + 1)
                    .copied()
                    .unwrap_or(top + ROW_HEIGHT);
                let centre = top + ROW_HEIGHT / 2.0;
                let node = Point::new(lane_x(row.lane), centre);

                let dashed = self.stashes.contains(&row.commit);

                for lane in &row.through {
                    line(
                        frame,
                        Point::new(lane_x(*lane), top),
                        Point::new(lane_x(*lane), bottom),
                        lane_colour(self.colours, *lane),
                        false,
                    );
                }

                for lane in &row.incoming {
                    elbow(
                        frame,
                        Point::new(lane_x(*lane), top),
                        node,
                        lane_colour(self.colours, *lane),
                        dashed,
                    );
                }

                for lane in &row.outgoing {
                    elbow(
                        frame,
                        Point::new(lane_x(*lane), bottom),
                        node,
                        lane_colour(self.colours, *lane),
                        dashed,
                    );
                }

                if self.labelled.contains(&row.commit) {
                    frame.fill_rectangle(
                        Point::new(0.0, centre - CONNECTOR_WIDTH / 2.0),
                        Size::new(node.x, CONNECTOR_WIDTH),
                        lane_colour(self.colours, row.lane),
                    );
                }

                if self.stashes.contains(&row.commit) {
                    draw_stash(
                        frame,
                        node,
                        lane_colour(self.colours, row.lane),
                        self.colours.inset,
                    );
                } else {
                    frame.fill(&canvas::Path::circle(node, FACE_RADIUS), self.colours.inset);

                    if let Some(author) = self.authors.get(index) {
                        match self.pictures.get(&author.fingerprint) {
                            Some(faces) => draw_picture(frame, node, &faces.round),
                            None => draw_mark(frame, node, &author.generated),
                        }
                    }

                    frame.stroke(
                        &canvas::Path::circle(node, FACE_RADIUS),
                        canvas::Stroke::default()
                            .with_color(lane_colour(self.colours, row.lane))
                            .with_width(RING_WIDTH),
                    );
                }
            }
        });

        vec![geometry]
    }
}

fn draw_stash(frame: &mut canvas::Frame, node: Point, colour: Color, inset: Color) {
    let side = FACE_RADIUS * 1.8;
    let corner = Point::new(node.x - side / 2.0, node.y - side / 2.0);
    let box_path = canvas::Path::rounded_rectangle(corner, Size::new(side, side), 2.0.into());

    frame.fill(&box_path, inset);

    let width = side * 0.5;
    let bar = FACE_RADIUS * 0.22;
    for step in 0..3 {
        let y = node.y - bar * 3.0 + step as f32 * bar * 2.5;
        frame.fill_rectangle(
            Point::new(node.x - width / 2.0, y),
            Size::new(width, bar),
            colour,
        );
    }

    frame.stroke(
        &box_path,
        canvas::Stroke {
            line_dash: canvas::LineDash {
                segments: DASH,
                offset: 0,
            },
            ..canvas::Stroke::default()
                .with_color(colour)
                .with_width(RING_WIDTH)
        },
    );
}

fn draw_picture(frame: &mut canvas::Frame, node: Point, picture: &image::Handle) {
    let bounds = Rectangle::new(
        Point::new(node.x - FACE_RADIUS, node.y - FACE_RADIUS),
        Size::new(FACE_RADIUS * 2.0, FACE_RADIUS * 2.0),
    );

    frame.draw_image(
        bounds,
        canvas::Image {
            border_radius: FACE_RADIUS.into(),
            ..canvas::Image::new(picture.clone())
        },
    );
}

fn draw_mark(frame: &mut canvas::Frame, node: Point, generated: &avatar::Generated) {
    let side = FACE_RADIUS * 2.0 * MARK_SHARE;
    let cell = side / avatar::GRID as f32;
    let origin = Point::new(node.x - side / 2.0, node.y - side / 2.0);
    let [red, green, blue] = generated.colour;
    let colour = Color::from_rgb8(red, green, blue);

    for (index, set) in generated.cells.iter().enumerate() {
        if !set {
            continue;
        }

        let (row, column) = (index / avatar::GRID, index % avatar::GRID);
        frame.fill_rectangle(
            Point::new(
                origin.x + column as f32 * cell,
                origin.y + row as f32 * cell,
            ),
            Size::new(cell, cell),
            colour,
        );
    }
}

impl Graph<'_> {
    fn draw_working_tree(&self, frame: &mut canvas::Frame) {
        let Some(first) = self.rows.first() else {
            return;
        };

        let lane = first.lane;
        let node = Point::new(lane_x(lane), ROW_HEIGHT / 2.0);

        line(
            frame,
            node,
            Point::new(lane_x(lane), ROW_HEIGHT + ROW_HEIGHT / 2.0),
            lane_colour(self.colours, lane),
            true,
        );

        frame.stroke(
            &canvas::Path::circle(node, FACE_RADIUS),
            canvas::Stroke {
                line_dash: canvas::LineDash {
                    segments: DASH,
                    offset: 0,
                },
                ..canvas::Stroke::default()
                    .with_color(lane_colour(self.colours, lane))
                    .with_width(LINE_WIDTH)
            },
        );
    }
}

pub fn lane_x(lane: usize) -> f32 {
    GRAPH_INSET + LANE_WIDTH / 2.0 + lane as f32 * LANE_WIDTH
}

pub fn lane_colour(colours: theme::Colours, lane: usize) -> Color {
    colours.lanes[lane % colours.lanes.len()]
}

fn line(frame: &mut canvas::Frame, from: Point, to: Point, colour: Color, dashed: bool) {
    draw(frame, &canvas::Path::line(from, to), colour, dashed);
}

/// Runs straight along the lane, turns a quarter at the height of the node, then runs level
/// into it, with the same corner radius at either end.
fn elbow(frame: &mut canvas::Frame, end: Point, node: Point, colour: Color, dashed: bool) {
    let across = node.x - end.x;
    if across.abs() < f32::EPSILON {
        line(frame, end, node, colour, dashed);
        return;
    }

    let down = node.y - end.y;
    let radius = CORNER_RADIUS.min(across.abs() / 2.0).min(down.abs() / 2.0);
    let (step, drop) = (across.signum() * radius, down.signum() * radius);

    let path = canvas::Path::new(|builder| {
        builder.move_to(end);
        builder.line_to(Point::new(end.x, node.y - drop));
        // A quadratic with the corner as its control point is a quarter turn of exactly
        // `radius`, and symmetrical either side of it.
        builder.quadratic_curve_to(Point::new(end.x, node.y), Point::new(end.x + step, node.y));
        builder.line_to(node);
    });

    draw(frame, &path, colour, dashed);
}

fn draw(frame: &mut canvas::Frame, path: &canvas::Path, colour: Color, dashed: bool) {
    let stroke = canvas::Stroke::default()
        .with_color(colour)
        .with_width(LINE_WIDTH);

    frame.stroke(
        path,
        if dashed {
            canvas::Stroke {
                line_dash: canvas::LineDash {
                    segments: DASH,
                    offset: 0,
                },
                ..stroke
            }
        } else {
            stroke
        },
    );
}
