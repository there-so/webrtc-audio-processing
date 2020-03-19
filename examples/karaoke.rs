// This example loops the microphone input back to the speakers, while applying echo cancellation,
// creating an experience similar to Karaoke microphones. It uses PortAudio as an interface to the
// underlying audio devices.
use ctrlc;
use failure::Error;
use portaudio;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use webrtc_audio_processing::{ffi::OptionalInt, *};

// The highest sample rate that webrtc-audio-processing supports.
const SAMPLE_RATE: f64 = 48_000.0;

// webrtc-audio-processing expects a 10ms chunk for each process call.
const FRAMES_PER_BUFFER: u32 = 480;

fn create_processor(
    num_capture_channels: i32,
    num_render_channels: i32,
) -> Result<Processor, Error> {
    let processor = Processor::new(&InitializationConfig {
        num_capture_channels,
        num_render_channels,
        ..InitializationConfig::default()
    })?;

    // High pass filter is a prerequisite to running echo cancellation.
    let config = Config {
        echo_cancellation: EchoCancellation {
            enable: true,
            suppression_level: EchoCancellation_SuppressionLevel::LOW,
            stream_delay_ms: OptionalInt { has_value: true, value: 0 },
        },
        enable_high_pass_filter: true,
        ..Config::default()
    };

    processor.set_config(&config);

    Ok(processor)
}

fn wait_ctrlc() -> Result<(), Error> {
    let running = Arc::new(AtomicBool::new(true));

    ctrlc::set_handler({
        let running = running.clone();
        move || {
            running.store(false, Ordering::SeqCst);
        }
    })?;

    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}

fn main() -> Result<(), Error> {
    // Stereo microphones.
    let input_channels = 2;
    // Stereo speakers.
    let output_channels = 2;

    let mut processor = create_processor(input_channels, output_channels)?;

    let pa = portaudio::PortAudio::new()?;

    let stream_settings = pa.default_duplex_stream_settings(
        input_channels,
        output_channels,
        SAMPLE_RATE,
        FRAMES_PER_BUFFER,
    )?;

    // Memory allocation should not happen inside the audio loop.
    let mut processed = vec![0f32; FRAMES_PER_BUFFER as usize * input_channels as usize];

    let mut stream = pa.open_non_blocking_stream(
        stream_settings,
        move |portaudio::DuplexStreamCallbackArgs { in_buffer, mut out_buffer, frames, .. }| {
            assert_eq!(frames as u32, FRAMES_PER_BUFFER);

            processed.copy_from_slice(&in_buffer);
            processor.process_capture_frame(&mut processed).unwrap();

            // Play back the processed audio capture.
            out_buffer.copy_from_slice(&processed);
            processor.process_render_frame(&mut out_buffer).unwrap();

            portaudio::Continue
        },
    )?;

    stream.start()?;

    wait_ctrlc()?;

    Ok(())
}
