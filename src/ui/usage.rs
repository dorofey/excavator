//! Process-only metrics: kernel sampling runs off the UI executor.
use super::*;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Sample {
    cpu: f64,
    memory: f64,
}
#[derive(Default)]
pub(super) struct UsageState {
    current: Option<Sample>,
    history: VecDeque<Sample>,
    open: bool,
    error: Option<String>,
}
impl UsageState {
    fn label(&self) -> String {
        self.current
            .map(|sample| format!("CPU {:.1}% · {:.0} MiB", sample.cpu, sample.memory))
            .unwrap_or_else(|| "CPU — · RAM —".into())
    }
}

fn process_usage() -> Result<(f64, f64), String> {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
            return Err(format!(
                "CPU sampling failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut task: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>();
        let result = libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            &mut task as *mut _ as *mut libc::c_void,
            size as i32,
        );
        if result != size as i32 {
            return Err(format!(
                "Memory sampling failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let cpu = usage.ru_utime.tv_sec as f64
            + usage.ru_utime.tv_usec as f64 / 1_000_000.
            + usage.ru_stime.tv_sec as f64
            + usage.ru_stime.tv_usec as f64 / 1_000_000.;
        Ok((cpu, task.pti_resident_size as f64 / 1_048_576.))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Process usage is available on macOS.".into())
    }
}

pub(super) struct UsageMonitor {
    usage: UsageState,
    colors: Tokens,
}
impl UsageMonitor {
    pub(super) fn new(colors: Tokens, cx: &mut Context<Self>) -> Self {
        let mut monitor = Self {
            usage: UsageState::default(),
            colors,
        };
        monitor.start_usage_poll(cx);
        monitor
    }
    pub(super) fn update_colors(&mut self, colors: Tokens, cx: &mut Context<Self>) {
        if self.colors != colors {
            self.colors = colors;
            cx.notify();
        }
    }
    pub(super) fn start_usage_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut previous: Option<(Instant, f64)> = None;
            loop {
                let reading = cx
                    .background_executor()
                    .spawn(async { (Instant::now(), process_usage()) })
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let previous_label = this.usage.label();
                        let previous_error = this.usage.error.clone();
                        match reading {
                            (time, Ok((cpu, memory))) => {
                                let percent = previous.map(|(last_time, last_cpu)| {
                                    ((cpu - last_cpu).max(0.)
                                        / time.duration_since(last_time).as_secs_f64().max(0.001))
                                        * 100.
                                });
                                previous = Some((time, cpu));
                                if let Some(cpu) = percent {
                                    let sample = Sample { cpu, memory };
                                    this.usage.current = Some(sample);
                                    if this.usage.open {
                                        this.usage.history.push_back(sample);
                                        while this.usage.history.len() > 120 {
                                            this.usage.history.pop_front();
                                        }
                                    }
                                }
                                this.usage.error = None;
                            }
                            (_, Err(error)) => {
                                this.usage.current = None;
                                this.usage.error = Some(error);
                                previous = None;
                            }
                        }
                        if this.usage.open
                            || previous_label != this.usage.label()
                            || previous_error != this.usage.error
                        {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();
    }
    pub(super) fn toggle(&mut self, cx: &mut Context<Self>) {
        self.usage.open = !self.usage.open;
        cx.notify();
    }
}
impl Render for UsageMonitor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self.usage.label();
        let entity = cx.entity().downgrade();
        let colors = self.colors;
        let cpu: Vec<f64> = if self.usage.open {
            self.usage.history.iter().map(|s| s.cpu).collect()
        } else {
            Vec::new()
        };
        let memory: Vec<f64> = if self.usage.open {
            self.usage.history.iter().map(|s| s.memory).collect()
        } else {
            Vec::new()
        };
        let memory_max = (memory.iter().copied().fold(64., f64::max) / 64.).ceil() * 64.;
        let cpu_max = (cpu.iter().copied().fold(100., f64::max) / 100.).ceil() * 100.;
        let error = self.usage.error.clone();
        gpui_kit::base::Popover::new("process-usage")
            .w(px(200.))
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .open(self.usage.open)
            .trigger(
                Button::new("process-usage-trigger")
                    .ghost()
                    .compact()
                    .w(px(200.))
                    .label(label)
                    .tooltip("Excavator CPU and memory · click to collect a graph"),
            )
            .on_open_change(move |open, _, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.usage.open = *open;
                    cx.notify();
                });
            })
            .content(move |_, _, _| {
                div()
                    .w(px(350.))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .bg(rgb(colors.dialog))
                    .text_color(rgb(colors.text))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .rounded_md()
                    .child("Excavator usage")
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(colors.muted))
                            .child("Sampling every 2 s · latest 120 samples"),
                    )
                    .child(usage_chart("CPU", cpu, cpu_max, "%", colors.accent, colors))
                    .child(usage_chart(
                        "Resident RAM (RSS)",
                        memory,
                        memory_max,
                        " MiB",
                        colors.warning,
                        colors,
                    ))
                    .when_some(error, |panel, error| {
                        panel.child(div().text_xs().text_color(rgb(colors.error)).child(error))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(colors.muted))
                            .child("100% CPU = one core. Terminal subprocesses excluded."),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(colors.muted))
                            .child("Collection pauses when closed; history stays in this session."),
                    )
            })
    }
}
impl Workspace {
    pub(super) fn status_usage(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        self.usage.update(cx, |monitor, cx| monitor.update_colors(colors, cx));
        self.usage.clone()
    }
    pub(super) fn toggle_usage(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.usage.update(cx, |monitor, cx| monitor.toggle(cx));
    }
}
fn usage_chart(
    title: &str,
    values: Vec<f64>,
    maximum: f64,
    unit: &str,
    color: u32,
    colors: Tokens,
) -> AnyElement {
    let latest = values
        .last()
        .map(|v| format!("{v:.1}{unit}"))
        .unwrap_or_else(|| "Waiting for samples…".into());
    let scale = format!("0–{maximum:.0}{unit}");
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .justify_between()
                .text_xs()
                .child(format!("{title} · {latest}"))
                .child(scale),
        )
        .child(
            canvas(
                move |_, _, _| (),
                move |bounds, _, window, _| {
                    let mut axis = PathBuilder::stroke(px(1.));
                    axis.move_to(point(bounds.left(), bounds.bottom()));
                    axis.line_to(point(bounds.right(), bounds.bottom()));
                    if let Ok(path) = axis.build() {
                        window.paint_path(path, rgb(colors.border));
                    }
                    if values.len() < 2 {
                        return;
                    }
                    let mut line = PathBuilder::stroke(px(1.5));
                    for (index, value) in values.iter().enumerate() {
                        let location = point(
                            bounds.left()
                                + bounds.size.width
                                    * (index as f32 / (values.len() - 1).max(1) as f32),
                            bounds.bottom()
                                - bounds.size.height * (*value / maximum).clamp(0., 1.) as f32,
                        );
                        if index == 0 {
                            line.move_to(location);
                        } else {
                            line.line_to(location);
                        }
                    }
                    if let Ok(path) = line.build() {
                        window.paint_path(path, rgb(color));
                    }
                },
            )
            .w_full()
            .h(px(70.)),
        )
        .into_any_element()
}
