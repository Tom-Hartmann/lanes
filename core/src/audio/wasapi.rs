//! WASAPI capture: endpoint loopback (system audio), input devices (mics) and
//! per-process loopback (Windows 10 2004+). Each capture runs on its own
//! MMCSS "Pro Audio" thread, event-driven, and delivers 48 kHz stereo f32
//! chunks stamped with the QPC time of their first frame.
//!
//! Devices that disappear or change (unplug, default-device switch, format
//! change) are reopened automatically; the owner is told via `on_status` so the
//! UI can show it instead of silently dropping audio.

use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient,
    IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    WAVEFORMATEXTENSIBLE_0, eCapture, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{BLOB, CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{HSTRING, Interface, implement};

use crate::Timestamp;
use crate::error::{Error, Result, WinContext};
use crate::util::{ComGuard, Priority, spawn};

/// What to capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureSpec {
    /// Loopback of an output device (`None` = the current default, followed on change).
    Loopback { device: Option<String> },
    /// An input device (`None` = the current default, followed on change).
    Input { device: Option<String> },
    /// Audio rendered by a process and its children.
    Process { pid: u32 },
}

/// 48 kHz interleaved stereo f32 samples whose first frame was captured at `time`.
pub struct Chunk {
    pub source: Arc<str>,
    pub time: Timestamp,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub enum CaptureStatus {
    Running { device: String },
    /// Device missing or failed; retrying.
    Waiting { reason: String },
}

pub struct SourceCapture {
    stop: Arc<AtomicBool>,
    reopen: Arc<AtomicBool>,
    thread: Option<JoinHandle<Option<()>>>,
}

impl SourceCapture {
    pub fn start(
        source: Arc<str>,
        spec: CaptureSpec,
        out: crossbeam_channel::Sender<Chunk>,
        on_status: impl Fn(&str, CaptureStatus) + Send + 'static,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let reopen = Arc::new(AtomicBool::new(false));
        let (stop_t, reopen_t) = (stop.clone(), reopen.clone());
        let thread = spawn(&format!("lanes-audio-{source}"), Priority::Audio, move || {
            let _com = ComGuard::mta();
            let mut backoff = Duration::from_millis(250);
            while !stop_t.load(Ordering::Relaxed) {
                match Stream::open(&spec) {
                    Ok(stream) => {
                        backoff = Duration::from_millis(250);
                        on_status(&source, CaptureStatus::Running { device: stream.label.clone() });
                        reopen_t.store(false, Ordering::Relaxed);
                        if let Err(e) = stream.run(&source, &out, &stop_t, &reopen_t) {
                            log::warn!("audio source {source}: {e}; reopening");
                            on_status(&source, CaptureStatus::Waiting { reason: e.to_string() });
                        }
                    }
                    Err(e) => {
                        on_status(&source, CaptureStatus::Waiting { reason: e.to_string() });
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(Duration::from_secs(3));
                    }
                }
            }
        })?;
        Ok(Self { stop, reopen, thread: Some(thread) })
    }

    /// Asks the capture to reopen its device (e.g. the default device changed).
    pub fn reopen(&self) {
        self.reopen.store(true, Ordering::Relaxed);
    }
}

impl Drop for SourceCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Event(HANDLE);
impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: handle from CreateEventW, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct Stream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: Event,
    label: String,
}

/// 48 kHz, stereo, 32-bit float.
fn float_format() -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: 32 },
        // SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT
        dwChannelMask: 0x3,
        SubFormat: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
    }
}

impl Stream {
    fn open(spec: &CaptureSpec) -> Result<Self> {
        let (client, loopback, label) = match spec {
            CaptureSpec::Loopback { device } => {
                let (c, name) = activate_endpoint(device.as_deref(), true)?;
                (c, true, name)
            }
            CaptureSpec::Input { device } => {
                let (c, name) = activate_endpoint(device.as_deref(), false)?;
                (c, false, name)
            }
            CaptureSpec::Process { pid } => (activate_process(*pid)?, true, format!("process {pid}")),
        };
        let format = float_format();
        let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        if loopback {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }
        // SAFETY: format outlives the call; the event handle is owned by `Event`.
        unsafe {
            client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &format.Format, None)
                .ctx("IAudioClient::Initialize")?;
            let event = Event(CreateEventW(None, false, false, None).ctx("CreateEventW")?);
            client.SetEventHandle(event.0).ctx("SetEventHandle")?;
            let capture: IAudioCaptureClient = client.GetService().ctx("IAudioCaptureClient")?;
            client.Start().ctx("IAudioClient::Start")?;
            Ok(Self { client, capture, event, label })
        }
    }

    fn run(&self, source: &Arc<str>, out: &crossbeam_channel::Sender<Chunk>, stop: &AtomicBool, reopen: &AtomicBool) -> Result<()> {
        let result = (|| -> Result<()> {
            while !stop.load(Ordering::Relaxed) {
                if reopen.load(Ordering::Relaxed) {
                    return Err(Error::Invalid("device change".into()));
                }
                // SAFETY: valid event handle.
                let woke = unsafe { WaitForSingleObject(self.event.0, 100) } == WAIT_OBJECT_0;
                if woke {
                    self.drain(source, out)?;
                }
            }
            Ok(())
        })();
        // SAFETY: stopping a started client.
        let _ = unsafe { self.client.Stop() };
        result
    }

    fn drain(&self, source: &Arc<str>, out: &crossbeam_channel::Sender<Chunk>) -> Result<()> {
        loop {
            // SAFETY: all out-pointers valid; every successful GetBuffer is
            // released with the same frame count before the next one.
            unsafe {
                let next = self.capture.GetNextPacketSize().ctx("GetNextPacketSize")?;
                if next == 0 {
                    return Ok(());
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                let mut qpc = 0u64;
                self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc)).ctx("GetBuffer")?;
                let n = frames as usize * 2;
                let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    vec![0.0f32; n]
                } else {
                    std::slice::from_raw_parts(data as *const f32, n).to_vec()
                };
                self.capture.ReleaseBuffer(frames).ctx("ReleaseBuffer")?;
                // qpcPosition is already in 100 ns units on the QPC timeline.
                let time = if qpc != 0 {
                    Timestamp(qpc as i64)
                } else {
                    Timestamp(Timestamp::now().0 - i64::from(frames) * Timestamp::TICKS_PER_SEC / 48_000)
                };
                // The mixer never blocks for long; if it did, dropping is better than stalling capture.
                let _ = out.try_send(Chunk { source: source.clone(), time, samples });
            }
        }
    }
}

fn activate_endpoint(device: Option<&str>, render: bool) -> Result<(IAudioClient, String)> {
    // SAFETY: COM is initialised on this thread (ComGuard); interfaces are owned.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ctx("MMDeviceEnumerator")?;
        let dev = match device {
            Some(id) => enumerator.GetDevice(&HSTRING::from(id)).ctx("GetDevice")?,
            None => enumerator.GetDefaultAudioEndpoint(if render { eRender } else { eCapture }, eConsole).ctx("default endpoint")?,
        };
        let name = super::devices::friendly_name(&dev).unwrap_or_else(|| "audio device".into());
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).ctx("IMMDevice::Activate")?;
        Ok((client, name))
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion(mpsc::SyncSender<()>, HANDLE);

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(&self, _op: windows::core::Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        // SAFETY: the event is alive until activate_process returns (it waits on it).
        unsafe { SetEvent(self.1) }
    }
}

/// Activates a process-loopback client (VAD\Process_Loopback) for `pid` and its child processes.
fn activate_process(pid: u32) -> Result<IAudioClient> {
    if !crate::audio::process_loopback_supported() {
        return Err(Error::Unsupported("per-application audio needs Windows 10 version 2004 or later".into()));
    }
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // A VT_BLOB PROPVARIANT pointing at `params`. Wrapped in ManuallyDrop so
    // PropVariantClear never tries to free our stack memory.
    let prop = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB { cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &mut params as *mut _ as *mut u8 },
                },
            }),
        },
    });
    // SAFETY: `params`/`prop` outlive the activation (we wait for completion
    // below); the event is closed by `Event` after the wait.
    unsafe {
        let event = Event(CreateEventW(None, true, false, None).ctx("CreateEventW")?);
        let (tx, _rx) = mpsc::sync_channel(1);
        let handler: IActivateAudioInterfaceCompletionHandler = Completion(tx, event.0).into();
        let op = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*prop), &handler)
            .ctx("ActivateAudioInterfaceAsync")?;
        if WaitForSingleObject(event.0, 5000) != WAIT_OBJECT_0 {
            return Err(Error::Unsupported("process loopback activation timed out".into()));
        }
        let mut hr = windows::core::HRESULT(0);
        let mut unknown = None;
        op.GetActivateResult(&mut hr, &mut unknown).ctx("GetActivateResult")?;
        hr.ok().ctx("process loopback activation")?;
        unknown.ok_or_else(|| Error::Unsupported("no audio client".into()))?.cast().ctx("IAudioClient")
    }
}
