//! Task Scheduler 2.0 (`ITaskService`) calls. Every `unsafe` block of the
//! admin feature lives here; the policy (which settings, which checks) is in
//! the parent module.

use std::path::Path;

use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, VARIANT_BOOL, VARIANT_FALSE, VARIANT_TRUE,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::TaskScheduler::{
    IExecAction, ILogonTrigger, IRegisteredTask, ITaskFolder, ITaskService, TASK_ACTION_EXEC,
    TASK_CREATE_OR_UPDATE, TASK_INSTANCES_IGNORE_NEW, TASK_LOGON_INTERACTIVE_TOKEN, TASK_RUNLEVEL_HIGHEST,
    TASK_TRIGGER_LOGON, TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HRESULT, Interface};

use crate::sys::com::ComApartment;

/// Include hidden tasks when enumerating a folder.
const TASK_ENUM_HIDDEN: i32 = 1;

/// Everything needed to register the elevated task.
pub struct TaskSpec<'a> {
    pub folder: &'a str,
    pub name: &'a str,
    pub exe: &'a Path,
    pub arguments: &'a str,
    pub working_dir: &'a Path,
    pub user_sid: &'a str,
    pub sddl: &'a str,
    pub execution_time_limit: &'a str,
    pub priority: i32,
    /// `Some(delay)` adds a logon trigger for `user_sid` with that ISO 8601 delay.
    pub logon_delay: Option<&'a str>,
    pub author: &'a str,
    pub description: &'a str,
}

/// A connection to the local Task Scheduler service.
pub struct Scheduler {
    // Declared before `_com`, so it is released while COM is still initialized.
    service: ITaskService,
    _com: ComApartment,
}

fn bstr(s: &str) -> BSTR {
    BSTR::from(s)
}

fn path_bstr(path: &Path) -> BSTR {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    BSTR::from_wide(&wide)
}

fn vbool(value: bool) -> VARIANT_BOOL {
    if value { VARIANT_TRUE } else { VARIANT_FALSE }
}

/// True for the "no such task / folder" errors Task Scheduler returns.
fn is_not_found(err: &windows::core::Error) -> bool {
    let code = err.code();
    code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
}

fn not_found_to_none<T>(result: windows::core::Result<T>) -> windows::core::Result<Option<T>> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

impl Scheduler {
    /// Initializes COM on this thread (if needed) and connects to the local service as the current user.
    pub fn connect() -> windows::core::Result<Self> {
        let com = ComApartment::init()?;
        // SAFETY: COM is initialized on this thread for as long as `com` lives,
        // and `com` is stored next to the interface it guards.
        let service: ITaskService = unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)? };
        let empty = VARIANT::default();
        // SAFETY: empty VARIANTs mean local machine, current user.
        unsafe { service.Connect(&empty, &empty, &empty, &empty)? };
        Ok(Self { service, _com: com })
    }

    fn root(&self) -> windows::core::Result<ITaskFolder> {
        // SAFETY: plain COM call on a live interface.
        unsafe { self.service.GetFolder(&bstr("\\")) }
    }

    /// The folder at `path`, or `None` if it does not exist.
    pub fn folder(&self, path: &str) -> windows::core::Result<Option<ITaskFolder>> {
        // SAFETY: plain COM call on a live interface.
        not_found_to_none(unsafe { self.service.GetFolder(&bstr(path)) })
    }

    /// The task `name` inside `folder`, or `None` if either does not exist
    /// (or is not visible to the caller).
    pub fn task(&self, folder: &str, name: &str) -> windows::core::Result<Option<IRegisteredTask>> {
        let Some(folder) = self.folder(folder)? else {
            return Ok(None);
        };
        // SAFETY: plain COM call on a live interface.
        not_found_to_none(unsafe { folder.GetTask(&bstr(name)) })
    }

    /// Creates or replaces the task described by `spec`.
    pub fn register(&self, spec: &TaskSpec) -> windows::core::Result<()> {
        // SAFETY: every call below is a plain COM call on a live interface
        // obtained from the previous one; BSTR/VARIANT arguments are owned
        // values that outlive each call.
        unsafe {
            let task = self.service.NewTask(0)?;

            let info = task.RegistrationInfo()?;
            info.SetAuthor(&bstr(spec.author))?;
            info.SetDescription(&bstr(spec.description))?;

            let principal = task.Principal()?;
            principal.SetUserId(&bstr(spec.user_sid))?;
            principal.SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN)?;
            principal.SetRunLevel(TASK_RUNLEVEL_HIGHEST)?;

            let settings = task.Settings()?;
            settings.SetExecutionTimeLimit(&bstr(spec.execution_time_limit))?;
            settings.SetDisallowStartIfOnBatteries(vbool(false))?;
            settings.SetStopIfGoingOnBatteries(vbool(false))?;
            settings.SetAllowDemandStart(vbool(true))?;
            settings.SetMultipleInstances(TASK_INSTANCES_IGNORE_NEW)?;
            settings.SetPriority(spec.priority)?;
            settings.SetStartWhenAvailable(vbool(false))?;

            if let Some(delay) = spec.logon_delay {
                let trigger: ILogonTrigger = task.Triggers()?.Create(TASK_TRIGGER_LOGON)?.cast()?;
                trigger.SetUserId(&bstr(spec.user_sid))?;
                trigger.SetDelay(&bstr(delay))?;
            }

            // Path and arguments are separate fields; nothing is concatenated
            // into a command line that could be re-parsed differently.
            let action: IExecAction = task.Actions()?.Create(TASK_ACTION_EXEC)?.cast()?;
            action.SetPath(&path_bstr(spec.exe))?;
            action.SetArguments(&bstr(spec.arguments))?;
            action.SetWorkingDirectory(&path_bstr(spec.working_dir))?;

            let sddl = VARIANT::from(spec.sddl);
            let folder = match self.folder(spec.folder)? {
                Some(folder) => folder,
                None => self.root()?.CreateFolder(&bstr(spec.folder), &sddl)?,
            };
            folder.RegisterTaskDefinition(
                &bstr(spec.name),
                &task,
                TASK_CREATE_OR_UPDATE.0,
                &VARIANT::from(spec.user_sid),
                &VARIANT::default(),
                TASK_LOGON_INTERACTIVE_TOKEN,
                &sddl,
            )?;
        }
        Ok(())
    }

    /// Deletes the task (if present), then the folder if nothing else is left in it.
    pub fn delete(&self, folder_path: &str, name: &str) -> windows::core::Result<()> {
        let Some(folder) = self.folder(folder_path)? else {
            return Ok(());
        };
        // SAFETY: plain COM calls on live interfaces with owned arguments.
        unsafe {
            if let Err(e) = folder.DeleteTask(&bstr(name), 0)
                && !is_not_found(&e)
            {
                return Err(e);
            }
            let tasks = folder.GetTasks(TASK_ENUM_HIDDEN)?.Count()?;
            let folders = folder.GetFolders(0)?.Count()?;
            if tasks == 0 && folders == 0 {
                drop(folder);
                if let Err(e) = self.root()?.DeleteFolder(&bstr(folder_path), 0)
                    && !is_not_found(&e)
                {
                    // The task is gone, which is what matters; an empty folder is harmless.
                    log::warn!("removing task folder {folder_path}: {e}");
                }
            }
        }
        Ok(())
    }
}

/// Starts a registered task now. Returns once the service accepted the request.
pub fn run(task: &IRegisteredTask) -> windows::core::Result<()> {
    // SAFETY: plain COM call on a live interface; an empty VARIANT means "no parameters".
    unsafe { task.Run(&VARIANT::default())? };
    Ok(())
}
