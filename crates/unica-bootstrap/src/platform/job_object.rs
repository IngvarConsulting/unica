//! Запуск рантайма внутри Job Object загрузчика.
//!
//! Ребёнок рождается приостановленным, входит в группу и только потом
//! получает первый квант. Иначе между `spawn` и `AssignProcessToJobObject`
//! остаётся окно: смерть загрузчика в нём оставляет рантайм сиротой, а сам
//! рантайм успевает породить потомков вне группы.
//!
//! Тот же приём живёт в `unica-coder` (`ProcessTree` в
//! `infrastructure/platform/process.rs`). Здесь — его минимальная копия:
//! `unica-coder` зависит от загрузчика, а не наоборот, и тянуть в загрузчик
//! управление деревьями ядра ради трёх вызовов незачем.
//!
//! Тесты живут в `process.rs`.

use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
};

/// Группа, которая гибнет вместе с последним своим дескриптором — то есть
/// вместе с загрузчиком, как бы он ни умер. Процесс, который сам просит выйти
/// из группы (живой агент раннера), выходит: отделение возможно, только если
/// его разрешает каждая группа над ним. Остальные гибнут вместе с группой.
pub(super) struct RuntimeJob(OwnedHandle);

impl RuntimeJob {
    fn create() -> io::Result<Self> {
        Self::create_with_limits(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK)
    }

    pub(super) fn create_pending() -> io::Result<Self> {
        Self::create_with_limits(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
    }

    fn create_with_limits(flags: u32) -> io::Result<Self> {
        // SAFETY: пустые атрибуты и имя дают безымянную группу по умолчанию;
        // дескриптор не наследуется.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: дескриптор только что создан и больше никем не владеется.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(job) });

        job.set_limits(flags)?;
        Ok(job)
    }

    pub(super) fn release_pending(&self) -> io::Result<()> {
        self.set_limits(JOB_OBJECT_LIMIT_BREAKAWAY_OK)
    }

    fn set_limits(&self, flags: u32) -> io::Result<()> {
        // SAFETY: нулевая инициализация допустима для этой POD-структуры Windows.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: указатель и размер соответствуют классу информации.
        let configured = unsafe {
            SetInformationJobObject(
                self.raw(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn terminate_until(&self, deadline: Instant) -> io::Result<()> {
        // SAFETY: the handle belongs to this startup Job.
        if unsafe { TerminateJobObject(self.raw(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        loop {
            // SAFETY: this POD buffer matches the queried information class.
            let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    self.raw(),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if accounting.ActiveProcesses == 0 {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "startup Job still contains processes",
                ));
            }
            std::thread::sleep(remaining.min(Duration::from_millis(5)));
        }
    }

    pub(super) fn raw(&self) -> HANDLE {
        self.0.as_raw_handle() as HANDLE
    }
}

/// Рантайм, уже включённый в группу и запущенный.
pub(super) struct SupervisedRuntime {
    pub(super) child: Child,
    /// Закрывается после ожидания: пока загрузчик жив, группа держит дерево.
    pub(super) _job: RuntimeJob,
}

impl SupervisedRuntime {
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

/// На каком шаге не удалось взять рантайм под надзор.
#[derive(Debug)]
pub(super) enum StartFailure {
    Job(io::Error),
    Spawn(io::Error),
    /// Ребёнок так и не начал работу: он завершён до первого кванта.
    Assign(io::Error),
    Resume(io::Error),
}

/// Как включить процесс в группу. Шов для теста: настоящий отказ
/// `AssignProcessToJobObject` на CI не вызвать.
pub(super) type Assign = fn(&RuntimeJob, &Child) -> io::Result<()>;

pub(super) fn assign_to_job(job: &RuntimeJob, child: &Child) -> io::Result<()> {
    // SAFETY: оба дескриптора живы на время вызова.
    if unsafe { AssignProcessToJobObject(job.raw(), child.as_raw_handle() as HANDLE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn spawn_supervised(
    mut command: Command,
    assign: Assign,
) -> Result<SupervisedRuntime, StartFailure> {
    let job = RuntimeJob::create().map_err(StartFailure::Job)?;
    command.creation_flags(CREATE_SUSPENDED);
    let mut child = command.spawn().map_err(StartFailure::Spawn)?;

    if let Err(error) = assign(&job, &child) {
        abandon(&mut child);
        return Err(StartFailure::Assign(error));
    }
    if let Err(error) = resume_primary_thread(child.id()) {
        abandon(&mut child);
        return Err(StartFailure::Resume(error));
    }
    Ok(SupervisedRuntime { child, _job: job })
}

/// `TerminateProcess` действует и на приостановленный процесс; ожидание
/// гарантирует, что к возврату ошибки его уже нет.
fn abandon(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// `std` закрывает дескриптор первичного потока сразу после `CreateProcessW`,
/// поэтому поток ищется снимком. Приостановленный процесс других потоков
/// создать не может, так что первый найденный поток — первичный.
fn resume_primary_thread(process_id: u32) -> io::Result<()> {
    // SAFETY: для TH32CS_SNAPTHREAD идентификатор процесса игнорируется.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: снимок только что создан и принадлежит только этой функции.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let snapshot = snapshot.as_raw_handle() as HANDLE;
    // SAFETY: нулевая инициализация допустима; размер задан ниже.
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    // SAFETY: снимок и запись удовлетворяют контракту ToolHelp.
    if unsafe { Thread32First(snapshot, &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    loop {
        if entry.th32OwnerProcessID == process_id {
            break;
        }
        // SAFETY: снимок и запись остаются действительными между вызовами.
        if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }

    // SAFETY: идентификатор потока получен из снимка; наследование выключено.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: дескриптор только что открыт и принадлежит только этой функции.
    let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
    // SAFETY: дескриптор открыт с правом THREAD_SUSPEND_RESUME.
    let previous = unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) };
    if previous == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    if previous != 1 {
        return Err(io::Error::other(format!(
            "unexpected primary thread suspend count: {previous}"
        )));
    }
    Ok(())
}
