use std::io::BufReader;
use std::path::PathBuf;

use clap::builder::PossibleValue;
use clap::{Arg, Command};

use hdpictureconverter::Image;

fn var_prefix_str(s: &str) -> Result<String, String> {
    let len = s.chars().count();
    if len != 2 {
        return Err(format!(
            "var_prefix must be exactly two characters, but is {}",
            len
        ));
    }

    for (i, c) in s.chars().enumerate() {
        if !c.is_ascii_alphabetic() {
            return Err(format!(
                "{:?} at var_prefix position {} is not an alphabetic character",
                c, i
            ));
        }
    }

    Ok(s.into())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum QuantizerChoice {
    LibImageQuant,
    NeuQuant,
}

impl clap::ValueEnum for QuantizerChoice {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::LibImageQuant, Self::NeuQuant]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Self::LibImageQuant => Some(PossibleValue::new("imagequant")),
            Self::NeuQuant => Some(PossibleValue::new("neuquant")),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let m = Command::new("HD picture converter")
        .args([
            Arg::new("image_file")
                .value_parser(clap::value_parser!(PathBuf))
                .required(true),
            Arg::new("var_prefix")
                .value_parser(var_prefix_str)
                .required(true),
            Arg::new("out_dir")
                .short('o')
                .long("outdir")
                .default_value(".")
                .value_parser(clap::value_parser!(PathBuf))
                .help("Write 8xv files to this directory"),
            Arg::new("quantizer")
                .short('z')
                .long("quantizer")
                .value_parser(clap::value_parser!(QuantizerChoice))
                .default_value("imagequant")
                .help("Select quantization algorithm"),
            Arg::new("quantizer_quality")
                .short('q')
                .long("quality")
                .value_parser(1..=30)
                .default_value("10")
                .help("Set quantizer quality: 1 is best (slowest) and 30 is worst (fastest)"),
        ])
        .get_matches();

    let image_file = m.get_one::<PathBuf>("image_file").unwrap();
    let var_prefix = m.get_one::<String>("var_prefix").unwrap();
    let out_dir = m.get_one::<PathBuf>("out_dir").unwrap();
    let quantizer = match m.get_one::<QuantizerChoice>("quantizer").unwrap() {
        QuantizerChoice::LibImageQuant => hdpictureconverter::QuantizerOption::Imagequant,
        QuantizerChoice::NeuQuant => hdpictureconverter::QuantizerOption::NeuQuant(
            *m.get_one::<i64>("quantizer_quality").unwrap() as i32,
        ),
    };

    let out_path = |filename: &str| -> PathBuf {
        let mut p = out_dir.clone();
        p.push(filename);
        p.set_extension("8xv");
        p
    };

    eprintln!("Opening image file {:?}", &image_file);
    let mut image = {
        let f = std::fs::File::open(&image_file)?;
        Image::new(
            BufReader::new(f),
            &image_file.file_name().unwrap().to_string_lossy(),
            var_prefix,
            quantizer,
        )
    }?;

    eprintln!("Quantizing..");
    image.quantize();

    // Write tiles
    eprint!("Writing tiles..");
    for tile in image.tiles() {
        let p = out_path(tile.appvar_name());

        eprint!(" {:?}", &p);
        let f = std::fs::File::create(p)?;
        tile.write_appvar(f)?;
    }
    eprintln!();

    // Write palette
    eprint!("Writing palette.. ");
    {
        let mut p = PathBuf::from(image.palette_appvar_name());
        p.set_extension("8xv");
        eprintln!("{:?}", &p);

        let f = std::fs::File::create(p)?;
        image.write_palette_appvar(f)?;
    }

    Ok(())
}
