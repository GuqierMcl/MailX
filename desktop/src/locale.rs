//! Platform locale detection stays in the desktop host.
use mailx_core::SupportedLocale;
use std::sync::OnceLock;

pub(crate) fn system_locale() -> SupportedLocale {
    static LOCALE: OnceLock<SupportedLocale> = OnceLock::new();
    *LOCALE.get_or_init(|| SupportedLocale::from_system_tag(sys_locale::get_locale().as_deref()))
}
