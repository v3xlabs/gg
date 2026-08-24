use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SIZE: u32 = 64;
pub const GRID: usize = 5;

pub struct Identity {
    pub name: String,
    pub email: String,
}

/// Drawn when there is no remote avatar, or before one has been fetched. Derived from the
/// identity alone, so an author always produces the same mark.
#[derive(Clone)]
pub struct Generated {
    pub colour: [u8; 3],
    pub cells: [bool; GRID * GRID],
}

#[derive(Debug)]
pub enum Error {
    Request(String),
    NotFound,
    Decode(String),
    Write(std::io::Error),
}

impl Identity {
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.name.trim().as_bytes());
        hasher.update([0]);
        hasher.update(self.email.trim().to_lowercase().as_bytes());
        hasher.finalize().into()
    }

    /// GitHub hands out `<id>+<login>@users.noreply.github.com`. The numeric id is enough
    /// to build an avatar URL, which avoids an API call and the authentication it needs.
    fn github_id(&self) -> Option<&str> {
        let local = self
            .email
            .trim()
            .to_lowercase()
            .ends_with("@users.noreply.github.com")
            .then(|| self.email.trim().split('@').next())??;

        let id = local.split_once('+').map_or(local, |(id, _)| id);
        id.chars().all(|c| c.is_ascii_digit()).then_some(id)
    }

    fn remote_url(&self) -> String {
        match self.github_id() {
            Some(id) => format!("https://avatars.githubusercontent.com/u/{id}?s={SIZE}"),
            None => {
                let mut hasher = Sha256::new();
                hasher.update(self.email.trim().to_lowercase().as_bytes());
                let digest = hex(&hasher.finalize());

                // d=404 so a missing gravatar is a 404 rather than their silhouette
                // placeholder, which would hide the generated mark behind it.
                format!("https://gravatar.com/avatar/{digest}?s={SIZE}&d=404")
            }
        }
    }
}

pub fn generated(identity: &Identity) -> Generated {
    let fingerprint = identity.fingerprint();

    let mut cells = [false; GRID * GRID];
    for row in 0..GRID {
        for column in 0..GRID.div_ceil(2) {
            let set = fingerprint[row * GRID + column] & 1 == 1;
            cells[row * GRID + column] = set;
            cells[row * GRID + (GRID - 1 - column)] = set;
        }
    }

    Generated {
        colour: colour_from(&fingerprint),
        cells,
    }
}

/// Saturation and lightness are fixed so every generated avatar reads at the same weight
/// against either theme; only the hue varies with the identity.
fn colour_from(fingerprint: &[u8; 32]) -> [u8; 3] {
    let hue = f32::from(fingerprint[0]) / 255.0 * 360.0;
    let (saturation, lightness): (f32, f32) = (0.55, 0.55);

    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let secondary = chroma * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let offset = lightness - chroma / 2.0;

    let (red, green, blue) = match hue as u32 / 60 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };

    [
        ((red + offset) * 255.0) as u8,
        ((green + offset) * 255.0) as u8,
        ((blue + offset) * 255.0) as u8,
    ]
}

pub fn cache_path(identity: &Identity) -> PathBuf {
    let directory = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"));

    directory
        .join(crate::config::ON_DISK_NAME)
        .join("avatars")
        .join(format!("{}.square.png", hex(&identity.fingerprint())))
}

pub fn cached(identity: &Identity) -> Option<PathBuf> {
    let path = cache_path(identity);
    path.is_file().then_some(path)
}

/// Blocking, and reaches the network.
pub fn fetch(identity: &Identity) -> Result<PathBuf, Error> {
    let mut response = ureq::get(&identity.remote_url())
        .call()
        .map_err(|error| match error {
            ureq::Error::StatusCode(404) => Error::NotFound,
            other => Error::Request(other.to_string()),
        })?;

    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(|error| Error::Request(error.to_string()))?;

    let decoded =
        image::load_from_memory(&bytes).map_err(|error| Error::Decode(error.to_string()))?;

    let path = cache_path(identity);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(Error::Write)?;
    }
    portrait(decoded)
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| Error::Decode(error.to_string()))?;

    Ok(path)
}

/// Cropped square and scaled, and nothing else: the window wants both shapes, so the round
/// mask waits for [`faces`].
fn portrait(picture: image::DynamicImage) -> image::RgbaImage {
    let side = picture.width().min(picture.height());

    picture
        .crop_imm(
            (picture.width() - side) / 2,
            (picture.height() - side) / 2,
            side,
            side,
        )
        .resize_exact(SIZE, SIZE, image::imageops::FilterType::Lanczos3)
        .to_rgba8()
}

pub struct Faces {
    pub size: u32,
    pub square: Vec<u8>,
    pub round: Vec<u8>,
}

/// The round copy is masked here rather than on disk so one cached file serves both shapes.
/// The fade over the last pixel keeps the rim from being a staircase.
pub fn faces(path: &Path) -> Option<Faces> {
    let square = image::open(path).ok()?.to_rgba8();
    let size = square.width().min(square.height());
    let mut round = square.clone();

    let centre = (size as f32 - 1.0) / 2.0;
    for (x, y, pixel) in round.enumerate_pixels_mut() {
        let distance = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
        let coverage = (centre + 0.5 - distance).clamp(0.0, 1.0);
        pixel[3] = (f32::from(pixel[3]) * coverage) as u8;
    }

    Some(Faces {
        size,
        square: square.into_raw(),
        round: round.into_raw(),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(error) => write!(formatter, "could not fetch the avatar: {error}"),
            Self::NotFound => formatter.write_str("no avatar is published for this address"),
            Self::Decode(error) => write!(formatter, "could not read the avatar: {error}"),
            Self::Write(error) => write!(formatter, "could not cache the avatar: {error}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(name: &str, email: &str) -> Identity {
        Identity {
            name: name.to_owned(),
            email: email.to_owned(),
        }
    }

    #[test]
    fn a_github_noreply_address_yields_an_avatar_url_without_an_api_call() {
        let author = identity("Luc", "1234567+luc@users.noreply.github.com");

        assert_eq!(author.github_id(), Some("1234567"));
        assert!(
            author
                .remote_url()
                .starts_with("https://avatars.githubusercontent.com/u/1234567")
        );
    }

    #[test]
    fn any_other_address_falls_back_to_gravatar() {
        let author = identity("Luc", "  Luc@Example.COM ");

        assert_eq!(author.github_id(), None);
        assert!(
            author
                .remote_url()
                .starts_with("https://gravatar.com/avatar/")
        );
        assert!(
            author.remote_url().contains("d=404"),
            "a missing gravatar has to read as missing, not as their placeholder"
        );
    }

    #[test]
    fn the_gravatar_digest_ignores_case_and_surrounding_space() {
        assert_eq!(
            identity("Luc", "luc@example.com").remote_url(),
            identity("Someone Else", " LUC@Example.com  ").remote_url(),
            "gravatar hashes the trimmed lowercased address, and nothing else"
        );
    }

    #[test]
    fn a_generated_avatar_is_stable_and_mirrored() {
        let first = generated(&identity("Luc", "luc@example.com"));
        let again = generated(&identity("Luc", "luc@example.com"));

        assert_eq!(first.cells, again.cells);
        assert_eq!(first.colour, again.colour);

        for row in 0..GRID {
            for column in 0..GRID {
                assert_eq!(
                    first.cells[row * GRID + column],
                    first.cells[row * GRID + (GRID - 1 - column)]
                );
            }
        }
    }

    #[test]
    fn different_authors_generate_different_avatars() {
        let one = generated(&identity("Luc", "luc@example.com"));
        let two = generated(&identity("Ada", "ada@example.com"));

        assert!(one.cells != two.cells || one.colour != two.colour);
    }
}
