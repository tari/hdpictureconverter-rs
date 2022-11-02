use std::io::{BufRead, Write};
use std::io::{Result as IoResult, Seek};
use std::iter::repeat;

use color_quant::NeuQuant;
use image::{GenericImageView, Pixel, Rgba, RgbaImage, SubImage};
use tifiles::VariableType;

pub struct Image {
    input: RgbaImage,
    quantizer: NeuQuant,
    var_prefix: String,
    name: String,
}

/// Compute the integer ceiling of `lhs / rhs`
fn div_ceil(lhs: u32, rhs: u32) -> u32 {
    (lhs + rhs - 1) / rhs
}

impl Image {
    const TILE_SIZE: u32 = 80;

    pub fn new<R: BufRead + Seek>(
        data: R,
        name: &str,
        var_prefix: &str,
        quantizer_quality: i32,
    ) -> IoResult<Self> {
        let loaded_image = match image::io::Reader::new(data).with_guessed_format()?.decode() {
            Ok(i) => i,
            Err(e) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Unable to decode image: {}", e),
                ))
            }
        };

        // Generate a black image that's rounded to a multiple of TILE_SIZE
        let mut image = image::ImageBuffer::from_pixel(
            div_ceil(loaded_image.width(), Self::TILE_SIZE) * Self::TILE_SIZE,
            div_ceil(loaded_image.height(), Self::TILE_SIZE) * Self::TILE_SIZE,
            Rgba([0u8, 0, 0, 255]),
        );

        // Paste the loaded image onto the black canvas, also blending down so we know we're fully
        // opaque.
        image::imageops::overlay(&mut image, &loaded_image, 0, 0);
        assert_eq!(
            image.height() % Self::TILE_SIZE,
            0,
            "Image height {} isn't a multiple of tile size",
            image.height()
        );
        assert_eq!(
            image.width() % Self::TILE_SIZE,
            0,
            "Image width {} isn't a multiple of tile size",
            image.width()
        );

        let quantizer = color_quant::NeuQuant::new(quantizer_quality, 254, &[]);

        assert_eq!(var_prefix.len(), 2);

        Ok(Image {
            input: image,
            quantizer,
            name: Self::generate_calc_name(name),
            var_prefix: var_prefix.to_string(),
        })
    }

    pub fn quantize(&mut self) {
        // Check that this is actually contiguous RGBA8, because the quantizer expects that format.
        let layout = self.input.sample_layout();
        assert_eq!(layout.channels, 4);
        assert_eq!(layout.channel_stride, 1);
        assert_eq!(layout.width_stride, 4);
        assert_eq!(
            layout.height_stride as u32,
            layout.width * layout.width_stride as u32
        );

        // This is suboptimal since we have fixed entries; avoid even feeding the quantizer
        // pixels corresponding to fixed palette entries.
        self.quantizer.init(self.input.as_flat_samples().as_slice());
    }

    /// Return an iterator over image [`Tile`]s.
    pub fn tiles(&self) -> Tiles {
        Tiles {
            full: self,
            next: (0, 0),
        }
    }

    pub fn width_tiles(&self) -> u32 {
        self.input.width() / Self::TILE_SIZE
    }

    pub fn height_tiles(&self) -> u32 {
        self.input.height() / Self::TILE_SIZE
    }

    fn generate_calc_name(s: &str) -> String {
        let mut chars = s.chars().filter(char::is_ascii_alphanumeric);

        // First character must be alphabetic, make it Z if not
        let head = chars
            .next()
            .map(|c| if !c.is_ascii_alphabetic() { 'Z' } else { c });

        // Recombine up to 8 chars, appending _ to pad if needed
        head.into_iter()
            .chain(chars)
            .chain(repeat('_'))
            .take(8)
            .collect()
    }

    pub fn palette_appvar_name(&self) -> String {
        format!("HP{:2}0000", self.var_prefix)
    }

    pub fn write_palette_appvar<W: Write + Seek>(&self, out: W) -> IoResult<W> {
        // The palette hard-codes black at index 0 and white at index 255
        let mut palette: Vec<(u8, u8, u8)> = vec![(0, 0, 0)];
        palette.extend(
            self.quantizer
                .color_map_rgb()
                .chunks_exact(3)
                .map(|s| (s[0], s[1], s[2])),
        );
        palette.push((255, 255, 255));
        assert_eq!(palette.len(), 256);

        let mut writer =
            tifiles::Writer::new(out, VariableType::AppVar, &self.palette_appvar_name(), true)?;

        // Header: signature, 8-character image name, 2-character var prefix
        // and index of last image tile.
        write!(
            writer,
            "HDPALV10{:8}{:2}{:03}{:03}",
            self.name,
            self.var_prefix,
            self.width_tiles() - 1,
            self.height_tiles() - 1,
        )?;

        // Palette data follows directly, little-endian RGB565
        for swatch in palette {
            writer.write_all(&GRGB1555::from(swatch).to_le_bytes())?;
        }
        writer.close()
    }
}

pub struct Tiles<'a> {
    full: &'a Image,
    next: (u32, u32),
}

/// One slice of an overall image.
///
/// Tiles each get saved into one appvar on the calculator.
pub struct Tile<'a> {
    /// The location of this tile in the overall image. (0, 0) is top-left.
    index: (u32, u32),
    /// The name of the appvar generated from this tile.
    appvar_name: String,
    full: &'a Image,
    content: SubImage<&'a RgbaImage>,
}

impl<'a> Tile<'a> {
    pub fn write_appvar<W: Write + Seek>(&self, out: W) -> IoResult<W> {
        let mut writer = tifiles::Writer::new(out, VariableType::AppVar, &self.appvar_name, true)?;

        // Image signature
        write!(writer, "HDPICCV4{:8}", &self.full.name)?;
        // Image dimensions, always the tile size
        writer.write_all(&[self.content.width() as u8, self.content.height() as u8])?;

        // Paletteized pixel data follows, one byte per pixel
        for y in 0..self.content.height() {
            for x in 0..self.content.width() {
                let color = self.content.get_pixel(x, y);

                // Palette index 0 and 255 are hard-coded
                let palette_index = if let &[0, 0, 0, _] = color.channels() {
                    0
                } else if let &[255, 255, 255, _] = color.channels() {
                    255
                } else {
                    // The quantizer is told to generate 254 colors because of the hard-coded
                    // entries, so add one so its output starts at 1.
                    1 + self.full.quantizer.index_of(color.channels())
                };

                debug_assert!((0..=255).contains(&palette_index));
                writer.write_all(&[palette_index as u8])?;
            }
        }

        writer.close()
    }

    pub fn index(&self) -> (u32, u32) {
        self.index
    }

    pub fn appvar_name(&self) -> &str {
        &self.appvar_name
    }
}

impl<'a> Iterator for Tiles<'a> {
    type Item = Tile<'a>;

    fn next(&mut self) -> Option<Tile<'a>> {
        let (x, y) = self.next;
        if y == self.full.height_tiles() {
            None
        } else {
            self.next.0 += 1;
            if self.next.0 >= self.full.width_tiles() {
                self.next = (0, y + 1);
            }

            Some(Tile {
                index: (x, y),
                appvar_name: format!("{:2}{:03}{:03}", self.full.var_prefix, x, y),
                full: &self.full,
                content: self.full.input.view(
                    x * Image::TILE_SIZE,
                    y * Image::TILE_SIZE,
                    Image::TILE_SIZE,
                    Image::TILE_SIZE,
                ),
            })
        }
    }
}

/// 16-bit color format natively used by the calculator.
///
/// This is a version of RGB565, with 5 bits allocated to each of the red and green channels
/// and 6 in green. The LSb of green is placed at bit 15, followed on downward by red, green and
/// blue.
///
/// ```text
///  ┌──────────────────────────────┐
///  │ ┌────────────┐ ┌────────────┐▼┌────────────┐
///  │ │            │ │            │ │            │
/// G0 R4 R3 R2 R1 R0 G5 G4 G3 G2 G1 B4 B3 B2 B1 B0
/// 15 14 13 12 11 10  9  8  7  6  5  4  3  2  1  0
/// ```
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct GRGB1555(u16);

/// Convert from 24-bit RGB.
impl From<(u8, u8, u8)> for GRGB1555 {
    fn from(rgb: (u8, u8, u8)) -> GRGB1555 {
        let r = (rgb.0 as f32 / 255.0 * 0b1_1111 as f32).round() as u16;
        let g = (rgb.1 as f32 / 255.0 * 0b11_1111 as f32).round() as u16;
        let b = (rgb.2 as f32 / 255.0 * 0b1_1111 as f32).round() as u16;

        GRGB1555(((g & 0b00_0001) << 15) | (r << 10) | ((g & 0b11_1110) << 4) | b)
    }
}

impl std::ops::Deref for GRGB1555 {
    type Target = u16;

    fn deref(&self) -> &u16 {
        &self.0
    }
}

#[test]
fn rgb_conversion_is_correct() {
    assert_eq!(GRGB1555::from((0xff, 0xff, 0xff)), GRGB1555(0xFFFF));
    assert_eq!(GRGB1555::from((0xff, 0, 0)), GRGB1555(0x7c00));
    assert_eq!(GRGB1555::from((0, 0xff, 0)), GRGB1555(0x83e0));
    assert_eq!(GRGB1555::from((0, 0, 0xff)), GRGB1555(0x001f));
    assert_eq!(GRGB1555::from((0x5d, 0x37, 0x2c)), GRGB1555(0x2CE5));
}
