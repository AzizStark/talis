//! Opt-in GPU timestamps. Never waits, adds frames, or changes rendering state
//! except briefly unbinding/restoring QUERY_BUFFER for a ready CPU-side read.
use crate::config::SceneVariant;
use anyhow::{Result, ensure};
use glow::HasContext;
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    rc::Rc,
    time::{Duration, Instant},
};

const POOL: usize = 8;
const MARKERS: usize = 16;
const READY_LIMIT: usize = 256;
const REPORT_LIMIT: usize = 512;
pub(super) type GetQueryIv = unsafe extern "system" fn(u32, u32, *mut i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum GpuStage {
    Arm,
    Record,
    Presentation,
    FullScene,
    Sleeve,
    Buttons,
    Knob,
    PreviousSleeve,
}
impl GpuStage {
    pub const ALL: [Self; 8] = [
        Self::Arm,
        Self::Record,
        Self::Presentation,
        Self::FullScene,
        Self::Sleeve,
        Self::Buttons,
        Self::Knob,
        Self::PreviousSleeve,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Arm => "arm",
            Self::Record => "dust_artwork",
            Self::Presentation => "presentation_copy",
            Self::FullScene => "full_scene",
            Self::Sleeve => "sleeve",
            Self::Buttons => "buttons",
            Self::Knob => "knob",
            Self::PreviousSleeve => "previous_sleeve",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    pub id: u64,
    pub dimensions: (i32, i32),
    pub scene: SceneVariant,
    pub submitted_at: Instant,
    pub transition: bool,
}
#[derive(Clone, Debug)]
pub struct GpuFrameTiming {
    pub info: FrameInfo,
    pub total_ms: f64,
    /// None means the pass did NOT run, not that it ran in zero time.
    pub stages_ms: [Option<f64>; 8],
}
#[derive(Default)]
struct Slot {
    queries: Vec<glow::NativeQuery>,
    info: Option<FrameInfo>,
    used: usize,
    spans: Vec<(GpuStage, usize, usize)>,
    valid: bool,
}
pub(super) struct GpuProfiler {
    gl: Rc<glow::Context>,
    bits: u32,
    detailed: bool,
    query_buffers: bool,
    slots: Vec<Slot>,
    pending: VecDeque<usize>,
    active: Option<usize>,
    sequence: u64,
    pub dropped: u64,
    pub invalid: u64,
    ready: VecDeque<GpuFrameTiming>,
    pub latest_ms: Option<f64>,
    report: Report,
}
impl GpuProfiler {
    pub(super) unsafe fn new(
        gl: Rc<glow::Context>,
        get_query_iv: GetQueryIv,
        detailed: bool,
    ) -> Result<Self> {
        let v = gl.version();
        ensure!(
            !v.is_embedded
                && ((v.major, v.minor) >= (3, 3)
                    || gl.supported_extensions().contains("GL_ARB_timer_query")),
            "GPU timestamps unsupported"
        );
        let mut bits = 0;
        unsafe {
            get_query_iv(glow::TIMESTAMP, glow::QUERY_COUNTER_BITS, &mut bits);
        }
        ensure!(
            (30..=64).contains(&bits),
            "GPU timestamp counter unavailable ({bits} bits)"
        );
        let query_buffers = (v.major, v.minor) >= (4, 4)
            || gl
                .supported_extensions()
                .contains("GL_ARB_query_buffer_object");
        let mut value = Self {
            gl,
            bits: bits as u32,
            detailed,
            query_buffers,
            slots: Vec::with_capacity(POOL),
            pending: VecDeque::with_capacity(POOL),
            active: None,
            sequence: 0,
            dropped: 0,
            invalid: 0,
            ready: VecDeque::new(),
            latest_ms: None,
            report: Report::default(),
        };
        for _ in 0..POOL {
            value.slots.push(Slot::default());
            for _ in 0..if detailed { MARKERS } else { 2 } {
                let query = unsafe { value.gl.create_query() }.map_err(anyhow::Error::msg)?;
                value.slots.last_mut().unwrap().queries.push(query);
            }
        }
        tracing::info!(
            detailed,
            counter_bits = bits,
            pool = POOL,
            "GPU timing enabled (asynchronous; renderer only)"
        );
        Ok(value)
    }
    fn stamp(&mut self) -> Option<usize> {
        let slot = &mut self.slots[self.active?];
        let marker = slot.used;
        if marker == slot.queries.len() {
            slot.valid = false;
            return None;
        }
        unsafe {
            self.gl.query_counter(slot.queries[marker], glow::TIMESTAMP);
        }
        slot.used += 1;
        Some(marker)
    }
    pub(super) fn begin_frame(&mut self, mut info: FrameInfo) {
        self.poll();
        debug_assert!(self.active.is_none());
        self.sequence += 1;
        info.id = self.sequence;
        let Some(index) = free_slot(self.slots.len(), &self.pending) else {
            self.dropped += 1;
            return;
        };
        let slot = &mut self.slots[index];
        slot.info = Some(info);
        slot.used = 0;
        slot.spans.clear();
        slot.valid = true;
        self.active = Some(index);
        self.stamp();
    }
    pub(super) fn begin_stage(&mut self) -> Option<usize> {
        if !self.detailed {
            return None;
        }
        let slot = &mut self.slots[self.active?];
        // Reserve a matching end marker AND the frame's final marker.
        if slot.used + 3 > slot.queries.len() {
            slot.valid = false;
            return None;
        }
        self.stamp()
    }
    pub(super) fn end_stage(&mut self, stage: GpuStage, begin: Option<usize>) {
        if let Some(begin) = begin {
            if let Some(end) = self.stamp() {
                self.slots[self.active.unwrap()]
                    .spans
                    .push((stage, begin, end));
            }
        }
    }
    pub(super) fn end_frame(&mut self, valid: bool) {
        if self.active.is_some() {
            self.stamp();
            let index = self.active.take().unwrap();
            self.slots[index].valid &= valid;
            self.pending.push_back(index);
        }
    }
    /// Read only completed timestamps. No render is queued to drain a paused
    /// frame: results can be collected on the next real frame or valid teardown.
    pub(super) fn poll(&mut self) {
        self.latest_ms = None;
        if self.pending.is_empty() {
            return;
        }
        // QUERY_RESULT_AVAILABLE also uses a CPU pointer through Glow. Clear
        // QUERY_BUFFER before BOTH availability checks and 64-bit reads.
        let buffer = if self.query_buffers {
            (unsafe { self.gl.get_parameter_i32(glow::QUERY_BUFFER_BINDING) }) as u32
        } else {
            0
        };
        if buffer != 0 {
            unsafe {
                self.gl.bind_buffer(glow::QUERY_BUFFER, None);
            }
        }
        while let Some(&index) = self.pending.front() {
            let slot = &self.slots[index];
            // ARB_timer_query guarantees earlier queries of the same type are
            // available when the last TIMESTAMP is available (section 6.1.6).
            if unsafe {
                self.gl.get_query_parameter_u32(
                    slot.queries[slot.used - 1],
                    glow::QUERY_RESULT_AVAILABLE,
                )
            } == 0
            {
                break;
            }
            self.pending.pop_front();
            if !slot.valid {
                self.invalid += 1;
                continue;
            }
            // Glow's u64 API accepts a pointer-as-offset. A bound query buffer
            // would interpret that address as a GPU-buffer offset, so preserve
            // and temporarily clear it where that feature exists.
            let mut times = [0_u64; MARKERS];
            for (i, value) in times.iter_mut().enumerate().take(slot.used) {
                unsafe {
                    self.gl.get_query_parameter_u64_with_offset(
                        slot.queries[i],
                        glow::QUERY_RESULT,
                        value as *mut u64 as usize,
                    );
                }
            }
            let mut frame = GpuFrameTiming {
                info: slot.info.unwrap(),
                total_ms: elapsed(times[0], times[slot.used - 1], self.bits),
                stages_ms: [None; 8],
            };
            for &(stage, begin, end) in &slot.spans {
                *frame.stages_ms[stage as usize].get_or_insert(0.0) +=
                    elapsed(times[begin], times[end], self.bits);
            }
            if !frame.total_ms.is_finite() || frame.total_ms > 1000.0 {
                self.invalid += 1;
                continue;
            }
            self.latest_ms = Some(frame.total_ms);
            if self.detailed {
                self.report.sample(&frame, self.dropped, self.invalid);
            }
            if self.ready.len() == READY_LIMIT {
                self.ready.pop_front();
            }
            self.ready.push_back(frame);
        }
        if let Some(buffer) = NonZeroU32::new(buffer) {
            unsafe {
                self.gl
                    .bind_buffer(glow::QUERY_BUFFER, Some(glow::NativeBuffer(buffer)));
            }
        }
    }
    pub(super) fn take(&mut self) -> Vec<GpuFrameTiming> {
        self.ready.drain(..).collect()
    }
}
fn free_slot(capacity: usize, pending: &VecDeque<usize>) -> Option<usize> {
    (0..capacity).find(|i| !pending.contains(i))
}
impl Drop for GpuProfiler {
    fn drop(&mut self) {
        // Owner drops only with the GL context current; context-loss handling
        // already forgets the entire Renderer rather than issuing GL deletes.
        self.poll();
        self.report.flush(self.dropped, self.invalid);
        tracing::info!(
            submitted = self.sequence,
            dropped = self.dropped,
            invalid = self.invalid,
            pending = self.pending.len(),
            "GPU profile final query counts (pending samples not waited for)"
        );
        for slot in &self.slots {
            for &query in &slot.queries {
                unsafe {
                    self.gl.delete_query(query);
                }
            }
        }
    }
}
fn elapsed(start: u64, end: u64, bits: u32) -> f64 {
    let delta = end.wrapping_sub(start);
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1_u64 << bits) - 1
    };
    (delta & mask) as f64 / 1_000_000.0
}

#[derive(Default)]
struct Report {
    origin: Option<Instant>,
    start: Option<Instant>,
    last: Option<Instant>,
    key: Option<((i32, i32), SceneVariant, bool)>,
    first_id: Option<u64>,
    last_id: u64,
    frames: usize,
    total: Vec<f64>,
    stages: [Vec<f64>; 8],
}
fn stats(values: &[f64]) -> (Option<f64>, Option<f64>, f64) {
    if values.is_empty() {
        return (None, None, 0.0);
    }
    let sum = values.iter().sum::<f64>();
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    (
        Some(sum / values.len() as f64),
        Some(sorted[((values.len() - 1) as f64 * 0.95).ceil() as usize]),
        sum,
    )
}
impl Report {
    fn sample(&mut self, frame: &GpuFrameTiming, dropped: u64, invalid: u64) {
        let now = frame.info.submitted_at;
        let origin = *self.origin.get_or_insert(now);
        if now.saturating_duration_since(origin) < Duration::from_secs(2) {
            return;
        }
        let key = (
            frame.info.dimensions,
            frame.info.scene,
            frame.info.transition,
        );
        if self.key.is_some_and(|old| old != key) {
            self.flush(dropped, invalid);
        }
        self.key = Some(key);
        self.start.get_or_insert(now);
        self.first_id.get_or_insert(frame.info.id);
        self.last_id = frame.info.id;
        self.last = Some(now);
        self.frames += 1;
        self.total.push(frame.total_ms);
        for (stage, value) in self.stages.iter_mut().zip(frame.stages_ms) {
            if let Some(value) = value {
                stage.push(value);
            }
        }
        if self.frames >= REPORT_LIMIT
            || now.saturating_duration_since(self.start.unwrap()) >= Duration::from_secs(2)
        {
            self.flush(dropped, invalid);
        }
    }
    fn flush(&mut self, dropped: u64, invalid: u64) {
        let Some((dimensions, scene, transition)) = self.key else {
            return;
        };
        if self.frames == 0 {
            return;
        }
        let seconds = self
            .last
            .unwrap()
            .saturating_duration_since(self.start.unwrap())
            .as_secs_f64();
        let (avg, p95, _) = stats(&self.total);
        tracing::info!(
            ?scene,
            width = dimensions.0,
            height = dimensions.1,
            transition,
            frames = self.frames,
            first_frame = self.first_id,
            last_frame = self.last_id,
            seconds,
            total_avg_ms = avg,
            total_p95_ms = p95,
            dropped,
            invalid,
            "GPU profile window (renderer only; not compositor/utilization)"
        );
        for stage in GpuStage::ALL {
            let values = &self.stages[stage as usize];
            let (avg, p95, sum) = stats(values);
            tracing::info!(
                stage = stage.name(),
                active_samples = values.len(),
                avg_ms = avg,
                p95_ms = p95,
                contribution_ms_per_frame = sum / self.frames as f64,
                measured_ms_per_second = (seconds > 0.0).then_some(sum / seconds),
                "GPU profile stage"
            );
        }
        self.start = None;
        self.last = None;
        self.key = None;
        self.first_id = None;
        self.frames = 0;
        self.total.clear();
        for stage in &mut self.stages {
            stage.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_pool_skips_sampling_without_reusing_pending_queries() {
        let mut pending = VecDeque::new();
        for expected in 0..POOL {
            let slot = free_slot(POOL, &pending).unwrap();
            assert_eq!(slot, expected);
            pending.push_back(slot);
        }
        assert_eq!(free_slot(POOL, &pending), None);
        pending.pop_front();
        assert_eq!(free_slot(POOL, &pending), Some(0));
    }
    #[test]
    fn timestamp_math_handles_64_bit_values_and_counter_wrap() {
        assert_eq!(elapsed(9_000_000_000, 9_000_250_000, 64), 0.25);
        assert_eq!(elapsed(u64::MAX - 99, 100, 64), 0.0002);
        assert_eq!(elapsed((1_u64 << 32) - 10, 10, 32), 0.00002);
    }
    #[test]
    fn statistics_distinguish_skipped_passes_from_zero_duration() {
        assert_eq!(stats(&[]), (None, None, 0.0));
        assert_eq!(stats(&[0.0]), (Some(0.0), Some(0.0), 0.0));
        assert_eq!(stats(&[3.0, 1.0, 2.0]), (Some(2.0), Some(3.0), 6.0));
    }
    #[test]
    fn reports_use_submission_time_and_separate_transitions_and_sizes() {
        let now = Instant::now();
        let mut report = Report::default();
        let mut frame = GpuFrameTiming {
            info: FrameInfo {
                id: 1,
                dimensions: (1920, 1080),
                scene: SceneVariant::Day,
                submitted_at: now,
                transition: false,
            },
            total_ms: 0.5,
            stages_ms: [None; 8],
        };
        report.sample(&frame, 0, 0);
        assert_eq!(report.frames, 0);
        frame.info.submitted_at = now + Duration::from_secs(2);
        frame.stages_ms[GpuStage::Presentation as usize] = Some(0.2);
        report.sample(&frame, 0, 0);
        assert_eq!(report.stages[GpuStage::Arm as usize].len(), 0);
        assert_eq!(report.frames, 1);
        frame.info.transition = true;
        report.sample(&frame, 0, 0);
        assert_eq!(report.frames, 1);
        assert!(report.key.unwrap().2);
        frame.info.dimensions = (3840, 2160);
        report.sample(&frame, 0, 0);
        assert_eq!(report.key.unwrap().0, (3840, 2160));
    }
    #[test]
    fn report_storage_is_bounded_even_with_no_time_advance() {
        let now = Instant::now();
        let mut report = Report {
            origin: Some(now - Duration::from_secs(2)),
            ..Default::default()
        };
        let mut frame = GpuFrameTiming {
            info: FrameInfo {
                id: 0,
                dimensions: (1920, 1080),
                scene: SceneVariant::Day,
                submitted_at: now,
                transition: false,
            },
            total_ms: 0.5,
            stages_ms: [Some(0.05); 8],
        };
        for id in 1..=REPORT_LIMIT {
            frame.info.id = id as u64;
            report.sample(&frame, 0, 0);
            assert!(report.total.len() < REPORT_LIMIT);
        }
        assert_eq!(report.frames, 0);
        assert!(report.total.is_empty());
        assert!(report.stages.iter().all(Vec::is_empty));
    }
}
