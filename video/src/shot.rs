//! Headless look check: the controls over a gray stand-in frame, written to a PNG,
//! no compositor or mpv needed. `vid --png OUT [--size WxH] [--pointer X,Y]`
//! (a pointer over the seek track shows its hover and time pill).

use std::time::Instant;

use frame::ui::State as UiState;
use frame::ui::Text;
use frame::ui::theme::FAMILY;
use frame::window::Canvas;

use crate::controls::{Controls, Look};
use crate::player::State;

/// Mid gray: where the picture would be.
const STAND_IN: u32 = 0xff40_4040;

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let val = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
    };
    let pair = |f: &str, sep: char| {
        val(f)
            .and_then(|s| s.split_once(sep))
            .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
    };
    let (w, h) = pair("--size", 'x').map_or((1280, 720), |(a, b)| (a as usize, b as usize));
    let out = val("--png").ok_or("--png needs a file")?;
    let mut ui = UiState::new(Text::new(FAMILY)?);
    ui.input.pos = pair("--pointer", ',').unwrap_or((-1.0, -1.0));
    let state = State {
        time: 754.0,
        duration: 3725.0,
        volume: 70.0,
        title: "sample".into(),
        ..State::default()
    };
    let look = Look {
        state: &state,
        visible: true,
        fullscreen: false,
    };
    let mut px = vec![STAND_IN; w * h];
    let mut controls = Controls::default();
    let t = Instant::now();
    // Several frames so hover springs settle as they would live.
    for _ in 0..40 {
        px.fill(STAND_IN);
        let mut f = ui.begin(Canvas { px: &mut px, w, h }, 0.05);
        controls.draw(&mut f, &look);
        f.end();
    }
    eprintln!("40 frames in {:?}", t.elapsed());
    frame::png::write(out, w, h, &px)?;
    Ok(())
}
