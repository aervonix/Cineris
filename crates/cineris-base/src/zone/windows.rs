//! The zone Windows is set to, through its own API, which knows each
//! year's rules (`GetDynamicTimeZoneInformation`,
//! `SystemTimeToTzSpecificLocalTimeEx`).
//!
//! The second of the two modules of this crate that may use `unsafe`: the
//! zone can only be read through the Windows API.

#![allow(
    unsafe_code,
    reason = "the system's time zone is reachable only through the Windows API"
)]

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, SystemTimeToTzSpecificLocalTimeEx,
};

use crate::time::{civil_from_days, days_from_civil};

/// What `GetDynamicTimeZoneInformation` returned, kept for every question.
#[derive(Clone)]
pub(super) struct SystemZone {
    info: DYNAMIC_TIME_ZONE_INFORMATION,
}

impl std::fmt::Debug for SystemZone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemZone").finish_non_exhaustive()
    }
}

/// `GetDynamicTimeZoneInformation`'s answer when it fails.
const TIME_ZONE_ID_INVALID: u32 = u32::MAX;

impl SystemZone {
    /// The zone the system is set to. `None` when the call fails.
    pub(super) fn read() -> Option<Self> {
        // SAFETY: the structure is plain data, for which all zeroes is a
        // valid value, and the call only writes into it.
        let mut info: DYNAMIC_TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a valid, writable structure of the right type.
        let answer = unsafe { GetDynamicTimeZoneInformation(&raw mut info) };
        (answer != TIME_ZONE_ID_INVALID).then_some(Self { info })
    }

    /// The offset at `secs` since the Unix epoch, in seconds east of UTC.
    pub(super) fn offset_at(&self, secs: i64) -> i32 {
        let days = secs.div_euclid(86_400);
        let in_day = secs.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        let Ok(year) = u16::try_from(year) else {
            return 0;
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a month, a day, an hour, a minute and a second are small and positive"
        )]
        let utc = SYSTEMTIME {
            wYear: year,
            wMonth: month as u16,
            wDayOfWeek: 0,
            wDay: day as u16,
            wHour: (in_day / 3600) as u16,
            wMinute: (in_day % 3600 / 60) as u16,
            wSecond: (in_day % 60) as u16,
            wMilliseconds: 0,
        };
        // SAFETY: as above, all zeroes is a valid SYSTEMTIME.
        let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
        // SAFETY: every pointer is to a valid structure of the right type,
        // the first two only read and the last only written.
        let ok = unsafe {
            SystemTimeToTzSpecificLocalTimeEx(&raw const self.info, &raw const utc, &raw mut local)
        };
        if ok == 0 {
            return 0;
        }
        let local_secs = days_from_civil(
            i64::from(local.wYear),
            i64::from(local.wMonth),
            i64::from(local.wDay),
        ) * 86_400
            + i64::from(local.wHour) * 3600
            + i64::from(local.wMinute) * 60
            + i64::from(local.wSecond);
        let utc_secs = days * 86_400 + in_day;
        i32::try_from(local_secs - utc_secs).unwrap_or(0)
    }
}
