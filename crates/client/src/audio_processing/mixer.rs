use std::{
  collections::{HashMap, VecDeque},
  sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
  },
};

#[derive(Default)]
struct Source {
  queue: VecDeque<f32>,
  primed: bool,
  last: f32,
}

#[derive(Clone)]
pub struct Mixer {
  sources: Arc<Mutex<HashMap<u32, Source>>>,
  target: usize,
  max: usize,
  // when set, output is silenced (queues still drain so we don't resume from a
  // stale backlog on undeafen). Shared with the voice actor.
  deafened: Arc<AtomicBool>,
}

// we have multiple incoming audio tracks from peers in a voice call,
// we'd like to mix them all into a single output track and play that
impl Mixer {
  pub fn new(sample_rate: u32, deafened: Arc<AtomicBool>) -> Self {
    Self {
      sources: Default::default(),
      target: sample_rate as usize * 40 / 1000, // 40ms prebuffer
      max: sample_rate as usize * 200 / 1000,   // 200ms
      deafened,
    }
  }

  pub fn push(&self, src: u32, samples: &[f32]) {
    let mut sources = self.sources.lock().unwrap();
    let source = sources.entry(src).or_default();
    source.queue.extend(samples);

    if source.queue.len() >= self.target {
      source.primed = true;
    }

    // drops packets from queue if we back up past 200 ms
    while source.queue.len() > self.max {
      source.queue.drain(..source.queue.len() - self.target);
    }
  }

  pub fn remove(&self, src: u32) {
    self.sources.lock().unwrap().remove(&src);
  }

  pub fn mix_mono(&self, out: &mut [f32]) {
    let mut sources = self.sources.lock().unwrap();
    // silence playback while deafened, but keep advancing the queues below so a
    // long-deafened call doesn't dump a buffered backlog the instant it's undone.
    let gain = if self.deafened.load(Ordering::Relaxed) {
      0.0
    } else {
      1.0
    };

    for sample in out.iter_mut() {
      *sample = gain
        * sources
        .values_mut()
        .map(|source| {
          if !source.primed {
            return 0.0; // silent until prefilling finished
          }
          match source.queue.pop_front() {
            Some(sample) => {
              source.last = sample;
              sample
            }
            None => {
              source.primed = false; // prefill again
              source.last *= 0.85;
              source.last
            }
          }
        })
        .sum::<f32>()
        .clamp(-1.0, 1.0);
    }
  }
}
