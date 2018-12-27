extern crate bus;
extern crate elapsed;
extern crate hound;
extern crate num;
extern crate sample;
extern crate time_calc;

use self::bus::BusReader;
use self::hound::WavReader;
use self::sample::frame::Stereo;
use self::sample::{Frame, Sample};

use audio::filters::{BiquadFilter, FilterOp, FilterType};
use audio::track_utils;

// // struct to help interpolation
// struct CubInterp {
//   iterp_val: f64,
//   n_1: Stereo<f32>,
//   n0: Stereo<f32>,
//   n1: Stereo<f32>,
//   n2: Stereo<f32>,
// }
// impl CubInterp {
//   // advance
//   fn next_source_frame(&mut self, f0: Stereo<f32>, f1: Stereo<f32>, f2: Stereo<f32>) {
//     self.n_1 = self.n0;
//     self.n0 = f0;
//     self.n1 = f1;
//     self.n2 = f2;
//   }
// /*
//     xm1 = x [inpos - 1];
//     x0  = x [inpos + 0];
//     x1  = x [inpos + 1];
//     x2  = x [inpos + 2];
//     a = (3 * (x0-x1) - xm1 + x2) / 2;
//     b = 2*x1 + xm1 - (5*x0 + x2) / 2;
//     c = (x1 - xm1) / 2;
//     y [outpos] = (((a * finpos) + b) * finpos + c) * finpos + x0;
//  */
//   fn interpolate(&mut self, x: f64) -> Stereo<f32> {
//     let x2 = x * x;
//     let mut out = self.n0.clone();
//     let mut chan = 0;
//     for c in out.iter_mut() {
//       let s_1 = self.n_1[chan].to_sample::<f64>();
//       let s0 = self.n0[chan].to_sample::<f64>();
//       let s1 = self.n1[chan].to_sample::<f64>();
//       let s2 = self.n2[chan].to_sample::<f64>();

//       let a0 = s2 - s1 - s_1 + s0; //p
//       let a1 = s_1 - s0 - a0;
//       let a2 = s1 - s_1;
//       let a3 = s0;

//       *c = (a0 * x * x2 + a1 * x2 + a2 * x + a3).to_sample::<f32>();
//       //
//       chan += 1;
//     }
//     out
//   }
// }

// struct to help interpolation
struct LinInterp {
  iterp_val: f64,
  left: Stereo<f32>,
  right: Stereo<f32>,
}
impl LinInterp {
  // advance
  fn next_source_frame(&mut self, frame: Stereo<f32>) {
    self.left = self.right;
    self.right = frame;
  }

  // Converts linearly from the previous value, using the next value to interpolate.
  fn interpolate(&mut self, x: f64) -> Stereo<f32> {
    self.left.zip_map(self.right, |l, r| {
      let l_f = l.to_sample::<f64>();
      let r_f = r.to_sample::<f64>();
      let diff = r_f - l_f;
      let out = ((diff * x) + l_f).to_sample::<f32>();
      out
    })
  }
}

// an audio track
pub struct RepitchAudioTrack {
  // commands rx
  command_rx: BusReader<::midi::CommandMessage>,
  // original tempo of the loaded audio
  original_tempo: f64,
  // playback_rate to match original_tempo
  playback_rate: f64,
  // the track is playring ?
  playing: bool,
  // volume of the track
  volume: f32,
  // original samples
  frames: Vec<Stereo<f32>>,
  // interpolation
  interpolation: LinInterp,
  // elapsed frames as requested by audio
  elapsed_frames: u64,
}
impl RepitchAudioTrack {
  // constructor
  pub fn new(command_rx: BusReader<::midi::CommandMessage>) -> RepitchAudioTrack {
    RepitchAudioTrack {
      command_rx,
      original_tempo: 120.0,
      playback_rate: 1.0,
      playing: false,
      volume: 0.5,
      frames: Vec::new(),
      interpolation: LinInterp {
        iterp_val: 0.0,
        left: Stereo::<f32>::equilibrium(),
        right: Stereo::<f32>::equilibrium(),
      },
      elapsed_frames: 0,
    }
  }

  // returns a buffer insead of frames one by one
  pub fn next_block(&mut self, size: usize) -> Vec<Stereo<f32>> {
    // non blocking command fetch
    self.fetch_commands();

    // doesnt consume if not playing
    if !self.playing {
      return (0..size).map(|_x| Stereo::<f32>::equilibrium()).collect();
    }

    /*
     * HERE WE CAN PROCESS BY CHUNK
     */
    // send full buffer
    return self.take(size).collect();
  }

  // load audio file
  pub fn load_file(&mut self, path: &str) {
    // load some audio
    let reader = WavReader::open(path).unwrap();

    // samples preparation
    let mut samples: Vec<f32> = reader
      .into_samples::<i16>()
      .filter_map(Result::ok)
      .map(i16::to_sample::<f32>)
      .collect();

    // parse and set original tempo
    let (orig_tempo, _beats) = track_utils::parse_original_tempo(path, samples.len());
    self.original_tempo = orig_tempo;

    // convert to stereo frames
    self.frames = track_utils::to_stereo(samples);

    // reset
    self.reset();
  }

  // upsampler next frame
  fn next_frame(&mut self) -> Stereo<f32> {
    let next_frame = self.frames[self.elapsed_frames as usize % self.frames.len()];
    self.elapsed_frames += 1;
    return next_frame;
  }

  // reset interp and counter
  fn reset(&mut self) {
    self.elapsed_frames = 0;
  }

  // fetch commands from rx, return true if received tick for latter sync
  fn fetch_commands(&mut self) {
    match self.command_rx.try_recv() {
      Ok(command) => match command {
        ::midi::CommandMessage::Playback(playback_message) => match playback_message.sync {
          ::midi::SyncMessage::Start() => {
            self.reset();
            self.playing = true;
          }
          ::midi::SyncMessage::Stop() => {
            self.playing = false;
            self.reset();
          }
          ::midi::SyncMessage::Tick(_tick) => {
            let rate = playback_message.time.tempo / self.original_tempo;
            // changed tempo
            if self.playback_rate != rate {
              self.playback_rate = rate;
            }
          }
        },
      },
      _ => (),
    };
  }
}

// Implement `Iterator` for `AudioTrack`.
impl Iterator for RepitchAudioTrack {
  type Item = Stereo<f32>;

  // next!
  fn next(&mut self) -> Option<Self::Item> {
    // advance frames
    while self.interpolation.iterp_val >= 1.0 {
      let f0 = self.next_frame();
      self.interpolation.next_source_frame(f0);
      self.interpolation.iterp_val -= 1.0;
    }

    // // apply interpolation
    let interp_val = self.interpolation.iterp_val;
    let mut next_i_frame = self.interpolation.interpolate(interp_val);
    self.interpolation.iterp_val += self.playback_rate;

    return Some(next_i_frame);
  }
}