use shared::SpinCtrlError;

const GROUP_GUIDANCE: &str =
    "Add yourself to the 'spinctrl' group and re-login: sudo usermod -a -G spinctrl $USER";

/// Translate a service-side error into a user-facing message. Permission
/// errors get actionable guidance about joining the `spinctrl` group (req
/// 8.9); everything else falls back to the error's Display.
pub fn explain_error(e: &SpinCtrlError) -> String {
    match e {
        SpinCtrlError::PermissionDenied(path) => {
            format!("Permission denied accessing {path}. {GROUP_GUIDANCE}")
        }
        SpinCtrlError::Io(io_err) if io_err.kind() == std::io::ErrorKind::PermissionDenied => {
            format!("Permission denied (EACCES). {GROUP_GUIDANCE}")
        }
        other => format!("{other}"),
    }
}