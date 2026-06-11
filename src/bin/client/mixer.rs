use std::{
  collections::{HashMap, VecDeque},
  sync::{Arc, Mutex},
};

#[derive(Default, Clone)]
pub struct Mixer(Arc<Mutex<HashMap<u32, VecDeque<f32>>>>);

// we have multiple incoming audio tracks from peers in a voice call,
// we'd like to mix them all into a single output track and play that
impl Mixer {
  pub fn push(&self, src: u32, samples: &[f32]) {
    let mut mixer = self.0.lock().unwrap();
    let queue = mixer.entry(src).or_default();
    queue.extend(samples);

    // drops packets from queue if we back up past 200 ms
    while queue.len() > 48_000 / 5 {
      queue.pop_front();
    }
  }

  pub fn remove(&self, src: u32) {
    self.0.lock().unwrap().remove(&src);
  }

  pub fn mix_into(&self, out: &mut [f32], channels: usize) {
    let mut m = self.0.lock().unwrap();
    for frame in out.chunks_mut(channels) {
      let s = m
        .values_mut()
        .filter_map(|q| q.pop_front())
        .sum::<f32>()
        .clamp(-1.0, 1.0);
      frame.fill(s);
    }
  }
}
