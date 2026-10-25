//! Spike: RNNoise vs DeepFilterNet3 (standard and low-latency) on speech + keyboard clicks.
//! Usage: dfspike <audio dir> <models dir>

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static A: Counting = Counting;

const SR: f32 = 48000.0;
const HOP: usize = 480;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
    fn noise(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }
}

/// One key event (press or release) added at `at`.
fn key_event(out: &mut [f32], at: usize, amp: f32, rng: &mut Rng) {
    let f_body = rng.range(1800.0, 5000.0);
    let f_low = rng.range(150.0, 320.0);
    let (tau_t, tau_b, tau_l) = (0.0005, rng.range(0.004, 0.009), 0.012);
    let res = |f: f32, tau: f32| {
        let r = (-1.0 / (tau * SR)).exp();
        let w = 2.0 * std::f32::consts::PI * f / SR;
        (2.0 * r * w.cos(), -r * r, 1.0 - r)
    };
    let (b1, b2, g) = res(f_body, tau_b);
    let (l1, l2, gl) = res(f_low, tau_l);
    let (mut y1, mut y2, mut z1, mut z2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let len = (0.06 * SR) as usize;
    for n in 0..len {
        let t = n as f32 / SR;
        let exc = if n < 96 { rng.noise() * (-t / tau_t).exp() } else { 0.0 };
        let y = b1 * y1 + b2 * y2 + exc;
        y2 = y1;
        y1 = y;
        let z = l1 * z1 + l2 * z2 + exc;
        z2 = z1;
        z1 = z;
        let s = exc * 0.6 + y * g * 6.0 + z * gl * 3.0;
        if let Some(o) = out.get_mut(at + n) {
            *o += s * amp;
        }
    }
}

/// Typing in bursts of words across the whole track.
fn typing(len: usize, rng: &mut Rng) -> Vec<f32> {
    let mut out = vec![0.0; len];
    let mut t = (0.3 * SR) as usize;
    while t < len {
        let keys = rng.range(3.0, 9.0) as usize;
        for _ in 0..keys {
            let amp = rng.range(0.6, 1.0);
            key_event(&mut out, t, amp, rng);
            key_event(&mut out, t + (rng.range(0.07, 0.14) * SR) as usize, amp * 0.5, rng);
            t += (rng.range(0.11, 0.25) * SR) as usize;
        }
        t += (rng.range(0.2, 1.2) * SR) as usize;
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let target = 10f32.powf(-14.0 / 20.0);
    out.iter_mut().for_each(|s| *s *= target / peak);
    out
}

trait Denoiser {
    fn name(&self) -> &str;
    fn hop(&mut self, input: &[f32], output: &mut [f32]);
}

struct Rn(Box<nnnoiseless::DenoiseState<'static>>, [f32; HOP], [f32; HOP]);
impl Denoiser for Rn {
    fn name(&self) -> &str {
        "RNNoise"
    }
    fn hop(&mut self, input: &[f32], output: &mut [f32]) {
        for (d, s) in self.1.iter_mut().zip(input) {
            *d = s * 32768.0;
        }
        self.0.process_frame(&mut self.2, &self.1);
        for (d, s) in output.iter_mut().zip(&self.2) {
            *d = s / 32768.0;
        }
    }
}

struct Dfn(String, DfTract, Array2<f32>, Array2<f32>);
impl Denoiser for Dfn {
    fn name(&self) -> &str {
        &self.0
    }
    fn hop(&mut self, input: &[f32], output: &mut [f32]) {
        self.2.as_slice_mut().unwrap().copy_from_slice(input);
        self.1.process(self.2.view(), self.3.view_mut()).unwrap();
        output.copy_from_slice(self.3.as_slice().unwrap());
    }
}

fn rms_db(x: &[f32]) -> f32 {
    let e = x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32;
    10.0 * (e + 1e-20).log10()
}

/// Lag (samples) at which `out` best matches `clean`.
fn best_lag(clean: &[f32], out: &[f32]) -> usize {
    let n = clean.len().min(out.len()) - 4800;
    (0..4800)
        .step_by(10)
        .max_by(|&a, &b| {
            let c = |lag: usize| (0..n).step_by(3).map(|i| clean[i] * out[i + lag]).sum::<f32>();
            c(a).total_cmp(&c(b))
        })
        .unwrap()
}

fn si_sdr(clean: &[f32], est: &[f32]) -> f32 {
    let dot: f32 = clean.iter().zip(est).map(|(a, b)| a * b).sum();
    let e: f32 = clean.iter().map(|a| a * a).sum();
    let a = dot / e;
    let (mut s, mut n) = (0.0f32, 0.0f32);
    for (c, x) in clean.iter().zip(est) {
        s += (a * c).powi(2);
        n += (x - a * c).powi(2);
    }
    10.0 * (s / n).log10()
}

fn write_wav(path: &Path, x: &[f32]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let audio = Path::new(&args[1]);
    let models = Path::new(&args[2]);

    let mut speech: Vec<f32> = hound::WavReader::open(audio.join("speech.wav"))
        .unwrap()
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();
    let peak = speech.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    speech.iter_mut().for_each(|s| *s *= 0.5 / peak);
    speech.truncate(speech.len() / HOP * HOP);
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let clicks = typing(speech.len(), &mut rng);
    let noisy: Vec<f32> = speech.iter().zip(&clicks).map(|(a, b)| a + b).collect();
    write_wav(&audio.join("noisy.wav"), &noisy);

    // Speech-free hops (typing only) and click-free speech hops.
    let hops = speech.len() / HOP;
    let speech_db: Vec<f32> = (0..hops).map(|h| rms_db(&speech[h * HOP..(h + 1) * HOP])).collect();
    let click_db: Vec<f32> = (0..hops).map(|h| rms_db(&clicks[h * HOP..(h + 1) * HOP])).collect();
    let near = |v: &[f32], h: usize, th: f32| (h.saturating_sub(3)..(h + 4).min(hops)).any(|k| v[k] > th);
    let gaps: Vec<usize> = (0..hops).filter(|&h| !near(&speech_db, h, -60.0) && click_db[h] > -60.0).collect();
    let clean_speech: Vec<usize> = (0..hops).filter(|&h| speech_db[h] > -35.0 && !near(&click_db, h, -80.0)).collect();
    println!(
        "{:.1} s audio, {} typing-only hops, {} click-free speech hops, input SI-SDR {:.1} dB\n",
        speech.len() as f32 / SR,
        gaps.len(),
        clean_speech.len(),
        si_sdr(&speech, &noisy)
    );

    let mut denoisers: Vec<Box<dyn Denoiser>> = vec![Box::new(Rn(
        nnnoiseless::DenoiseState::new(),
        [0.0; HOP],
        [0.0; HOP],
    ))];
    for (name, dir) in [("DFN3", "DeepFilterNet3_onnx"), ("DFN3-LL", "DeepFilterNet3_ll_onnx")] {
        let t0 = Instant::now();
        let tar = models.join(format!("../dl/DeepFilterNet-main/models/{dir}.tar.gz"));
        let model = DfTract::new(DfParams::new(tar).unwrap(), &RuntimeParams::default_with_ch(1)).unwrap();
        println!("{name}: model built in {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0);
        denoisers.push(Box::new(Dfn(name.into(), model, Array2::zeros((1, HOP)), Array2::zeros((1, HOP)))));
    }
    println!();

    for d in &mut denoisers {
        let mut out = vec![0.0f32; noisy.len()];
        let mut times = Vec::with_capacity(hops);
        let mut steady_allocs = 0;
        for h in 0..hops {
            let r = h * HOP..(h + 1) * HOP;
            let warm = h >= 100;
            ALLOCS.store(0, Ordering::Relaxed);
            COUNTING.store(warm, Ordering::Relaxed);
            let t = Instant::now();
            d.hop(&noisy[r.clone()], &mut out[r]);
            let dt = t.elapsed();
            COUNTING.store(false, Ordering::Relaxed);
            if warm {
                steady_allocs += ALLOCS.load(Ordering::Relaxed);
            }
            times.push(dt.as_secs_f64() * 1e6);
        }
        let lag = best_lag(&speech, &out);
        let aligned: Vec<f32> = out[lag..].to_vec();
        let n = aligned.len();
        let (s, a) = (&speech[..n], &aligned[..]);
        let seg = |idx: &[usize], x: &[f32]| -> f32 {
            let v: Vec<f32> = idx.iter().filter(|&&h| (h + 1) * HOP <= n).flat_map(|&h| x[h * HOP..(h + 1) * HOP].to_vec()).collect();
            rms_db(&v)
        };
        times.sort_by(f64::total_cmp);
        let p = |q: f64| times[((times.len() as f64 - 1.0) * q) as usize];
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        println!("{}:", d.name());
        println!(
            "  CPU per 10 ms hop: mean {:.0} us, p50 {:.0}, p99 {:.0}, max {:.0}  (= {:.1}% of one core)",
            mean,
            p(0.5),
            p(0.99),
            p(1.0),
            mean / 100.0
        );
        println!("  allocations after warm-up: {} over {} hops", steady_allocs, hops - 100);
        println!("  delay: {:.1} ms", lag as f32 / SR * 1000.0);
        println!(
            "  typing-only parts: {:.1} dB quieter (in {:.1} dBFS -> out {:.1} dBFS)",
            seg(&gaps, &noisy[..n]) - seg(&gaps, a),
            seg(&gaps, &noisy[..n]),
            seg(&gaps, a)
        );
        println!(
            "  click-free speech: {:+.1} dB vs clean speech",
            seg(&clean_speech, a) - seg(&clean_speech, s)
        );
        println!("  SI-SDR vs clean speech: {:.1} dB\n", si_sdr(s, a));
        let file = d.name().to_lowercase().replace('-', "_");
        write_wav(&audio.join(format!("out_{file}.wav")), &out);
    }
}
