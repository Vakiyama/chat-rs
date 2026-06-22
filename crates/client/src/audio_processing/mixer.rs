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
  // whether this source has prebuffered at least once. The first prime waits for
  // the full `target` cushion; after an underrun we re-prime at the smaller
  // `resume` threshold so a brief hiccup doesn't cost a full `target` of silence.
  ever_primed: bool,
  last: f32,
}

#[derive(Clone)]
pub struct Mixer {
  sources: Arc<Mutex<HashMap<u32, Source>>>,
  target: usize,
  resume: usize,
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
      target: sample_rate as usize * 40 / 1000, // 40ms initial prebuffer
      resume: sample_rate as usize * 10 / 1000, // 10ms re-prime after an underrun
      max: sample_rate as usize * 200 / 1000,   // 200ms
      deafened,
    }
  }

  pub fn push(&self, src: u32, samples: &[f32]) {
    // tolerate a poisoned lock (a panic in another holder) rather than panicking
    // again — this runs in the realtime output callback on some paths.
    let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
    let source = sources.entry(src).or_default();
    source.queue.extend(samples);

    // wait for the full cushion on first start; after an underrun a smaller
    // cushion is enough to resume, so a momentary gap doesn't mute for 40ms.
    let threshold = if source.ever_primed {
      self.resume
    } else {
      self.target
    };
    if source.queue.len() >= threshold {
      source.primed = true;
      source.ever_primed = true;
    }

    // drops packets from queue if we back up past 200 ms
    while source.queue.len() > self.max {
      source.queue.drain(..source.queue.len() - self.target);
    }
  }

  pub fn remove(&self, src: u32) {
    self
      .sources
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .remove(&src);
  }

  pub fn mix_mono(&self, out: &mut [f32]) {
    // tolerate a poisoned lock (a panic in another holder) rather than panicking
    // again — this runs in the realtime output callback on some paths.
    let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
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
              source.primed = false; // prefill again (at the smaller resume cushion)
              // gentle fade of the last sample instead of an abrupt cut: ~0.99
              // per sample decays to near-silence over a few ms, avoiding the
              // click a fast (0.85/sample) drop produced on every underrun.
              source.last *= 0.99;
              source.last
            }
          }
        })
        .sum::<f32>()
        .clamp(-1.0, 1.0);
    }
  }
}
