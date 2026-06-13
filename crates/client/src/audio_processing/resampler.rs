use rubato::Resampler as _;

pub struct Resampler {
  inner: Option<rubato::FftFixedIn<f32>>,
  chunk_in: usize,
  buf: Vec<f32>,
}

impl Resampler {
  pub fn new(in_rate: u32, out_rate: u32) -> anyhow::Result<Self> {
    if in_rate == out_rate {
      return Ok(Self {
        inner: None,
        chunk_in: 0,
        buf: Vec::new(),
      });
    }
    let chunk_in = 1024;
    let inner = rubato::FftFixedIn::<f32>::new(
      in_rate as usize,
      out_rate as usize,
      chunk_in,
      2,
      1, // mono
    )?;
    Ok(Self {
      inner: Some(inner),
      chunk_in,
      buf: Vec::new(),
    })
  }

  pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> anyhow::Result<()> {
    let Some(rs) = self.inner.as_mut() else {
      out.extend_from_slice(input); // passthrough
      return Ok(());
    };
    self.buf.extend_from_slice(input);
    while self.buf.len() >= self.chunk_in {
      let chunk: Vec<f32> = self.buf.drain(..self.chunk_in).collect();
      let waves = rs.process(&[chunk], None)?; // &[Vec<f32>], one ch
      out.extend_from_slice(&waves[0]);
    }
    Ok(())
  }
}
