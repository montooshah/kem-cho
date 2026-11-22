//! Transcribes WAV files through Kem Cho's real pipeline with weights from any file:
//! `eval <catalog-model-id> <weights.bin> <language> <wav>...`
use std::path::Path;

use kemcho_core::transcribe::Options;
use kemcho_core::{Transcriber, models, vad, wav};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let transcriber = Transcriber::load_file(models::find(&args[0])?, Path::new(&args[1]))?;
    let opts = Options {
        language: &args[2],
        ..Options::default()
    };
    for file in &args[3..] {
        let audio = wav::read_16k_mono(Path::new(file))?;
        let speech = vad::trim(&audio, vad::DEFAULT_MIN_RMS).unwrap_or(&audio);
        let out = transcriber.transcribe(speech, &opts)?;
        println!(
            "{file}\t{}\t{}",
            out.elapsed_ms,
            out.text.replace('\t', " ")
        );
    }
    Ok(())
}
