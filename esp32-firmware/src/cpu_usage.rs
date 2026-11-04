
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use esp_hal::{system::Cpu, time::Instant};
use rtos_trace::{RtosTrace, TaskInfo};

static IDLE_ACTIVE: AtomicBool = AtomicBool::new(false);
static IDLE_STARTED_US: AtomicU32 = AtomicU32::new(0);
static IDLE_ACCUMULATED_US: AtomicU32 = AtomicU32::new(0);

pub struct CpuTrace;

fn now_us() -> u32 {
    Instant::now().duration_since_epoch().as_micros() as u32
}

fn on_cpu0() -> bool {
    Cpu::current() == Cpu::ProCpu
}

fn enter_idle() {
    if !on_cpu0() {
        return;
    }

    if !IDLE_ACTIVE.swap(true, Ordering::Relaxed) {
        IDLE_STARTED_US.store(now_us(), Ordering::Relaxed);
    }
}

fn leave_idle() {
    if !on_cpu0() {
        return;
    }

    if IDLE_ACTIVE.swap(false, Ordering::Relaxed) {
        let start = IDLE_STARTED_US.load(Ordering::Relaxed);
        let elapsed = now_us().wrapping_sub(start);

        IDLE_ACCUMULATED_US.fetch_add(elapsed, Ordering::Relaxed);
    }
}

// Read accumulated idle time between samples.
// Call this from the CPU 0 telemetry task.
pub fn take_idle_us() -> u32 {
    IDLE_ACCUMULATED_US.swap(0, Ordering::Relaxed)
}

rtos_trace::global_trace! { CpuTrace }

impl RtosTrace for CpuTrace {
    fn start() {}
    fn stop() {}

    fn task_new(_id: u32) {}
    fn task_send_info(_id: u32, _info: TaskInfo) {}
    fn task_new_stackless(
        _id: u32,
        _name: &'static str,
        _priority: u32,
    ) {}
    fn task_terminate(_id: u32) {}

    fn task_exec_begin(_id: u32) {
        leave_idle();
    }

    fn task_exec_end() {}

    fn task_ready_begin(_id: u32) {}
    fn task_ready_end(_id: u32) {}

    fn system_idle() {
        enter_idle();
    }

    fn isr_enter() {}
    fn isr_exit() {}
    fn isr_exit_to_scheduler() {}

    fn name_marker(_id: u32, _name: &'static str) {}
    fn marker(_id: u32) {}
    fn marker_begin(_id: u32) {}
    fn marker_end(_id: u32) {}
}
