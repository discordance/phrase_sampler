extern crate bus;
extern crate hound;
extern crate num;
extern crate sample;
extern crate time_calc;
extern crate aubio;

use self::bus::BusReader;
use self::hound::WavReader;
use self::sample::frame::Stereo;
use self::sample::{Frame, Sample};
use self::aubio::pvoc::Pvoc;

use audio::track_utils;

const HOP_SIZE : usize = 512;
const WIND_SIZE : usize = 2048;

// an audio track
pub struct PvocAudioTrack {
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
  // elapsed frames as requested by audio
  elapsed_frames: u64,
}

impl PvocAudioTrack {
  // constructor
  pub fn new(command_rx: BusReader<::midi::CommandMessage>) -> PvocAudioTrack {
    
    let mut aubio_pvoc = Pvoc::new(WIND_SIZE, HOP_SIZE).expect("Pvoc::new");

    PvocAudioTrack {
      command_rx,
      original_tempo: 120.0,
      playback_rate: 1.0,
      playing: false,
      volume: 0.5,
      frames: Vec::new(),
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

    // send full buffer
    return self.take(size).collect();
  }

  // load audio file
  pub fn load_file(&mut self, path: &str) {
    // load some audio
    let reader = WavReader::open(path).unwrap();

    // samples preparation
    let samples: Vec<f32> = reader
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

  // just iterate into the frame buffer
  fn next_frame(&mut self) -> Stereo<f32> {
    // grab next frame in the frames buffer
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
impl Iterator for PvocAudioTrack {
  type Item = Stereo<f32>;

  // next!
  fn next(&mut self) -> Option<Self::Item> {
    // non blocking command fetch
    self.fetch_commands();

    // doesnt consume if not playing
    if !self.playing {
      return Some(Stereo::<f32>::equilibrium());
    }

    // gte next frame
    let next_frame = self.next_frame();

    // return
    return Some(next_frame);
    /*
     * HERE WE CAN PROCESS BY FRAME
     */
    // FILTER BANK
    // let frame = self.filter_bank.process(frame);
  }
}
