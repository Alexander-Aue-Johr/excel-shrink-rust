//! Compare small JSON writes into gzip with and without a preceding buffer.
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use std::io::{BufWriter, Read, Write};
use std::time::Instant;

fn rows(output: &mut impl Write, digests: &[String]) -> std::io::Result<()> {
    serde_json::to_writer(
        &mut *output,
        &serde_json::json!({"schema":1,"contexts":["before","after"],
            "columns":["sheet","location","before","after","context_before","context_after","also_in_original_open_save"]}),
    )?;
    output.write_all(b"\n")?;
    for (index, digest) in digests.iter().enumerate() {
        serde_json::to_writer(
            &mut *output,
            &("Data", format!("cell:A{}", index + 1), digest, digest, 0, 1, false),
        )?;
        output.write_all(b"\n")?;
    }
    Ok(())
}

fn encode(digests: &[String], buffered: bool) -> std::io::Result<(Vec<u8>, f64)> {
    let started = Instant::now();
    let encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let bytes = if buffered {
        let mut output = BufWriter::new(encoder);
        rows(&mut output, digests)?;
        output.into_inner().map_err(|error| error.into_error())?.finish()?
    } else {
        let mut output = encoder;
        rows(&mut output, digests)?;
        output.finish()?
    };
    Ok((bytes, started.elapsed().as_secs_f64()))
}

fn decode(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut result = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut result)?;
    Ok(result)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = std::env::args().nth(1).unwrap_or_else(|| "100000".into()).parse::<usize>()?;
    let digests: Vec<_> = (0..count)
        .map(|index| format!("{:x}", Sha256::digest(index.to_le_bytes())))
        .collect();
    for trial in 0..4 {
        let order = if trial % 2 == 0 { [false, true] } else { [true, false] };
        let mut results = Vec::new();
        for buffered in order {
            let (bytes, seconds) = encode(&digests, buffered)?;
            results.push((buffered, bytes, seconds));
        }
        let left = decode(&results[0].1)?;
        let right = decode(&results[1].1)?;
        assert_eq!(left, right, "Audit payload changed");
        println!("{}", serde_json::json!({
            "trial":trial,"rows":count,"decoded_bytes":left.len(),
            "decoded_sha256":format!("{:x}", Sha256::digest(&left)),
            "runs":results.iter().map(|(buffered, bytes, seconds)|
                serde_json::json!({"buffered":buffered,"compressed_bytes":bytes.len(),"seconds":seconds}))
                .collect::<Vec<_>>()
        }));
    }
    Ok(())
}
