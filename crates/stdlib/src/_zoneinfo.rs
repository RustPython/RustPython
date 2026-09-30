//! Native `_zoneinfo`.
//!
//! TZif bytes are decoded by `zoneinfo._common.load_data`. Transition rules,
//! `fromutc`, and the zone cache live here.

// cspell:ignore ttinfo ttinfos utcoff dstoff fromutc stdoff tzstr tzpath isdst isdsts tzrule TZif

pub(crate) use _zoneinfo::module_def;

#[pymodule]
mod _zoneinfo {
    use alloc::collections::VecDeque;
    use std::collections::HashMap;

    #[cfg(not(feature = "threading"))]
    use core::cell::RefCell;
    #[cfg(feature = "threading")]
    use std::sync::{Mutex, OnceLock};

    use crate::_datetime::{PyTzInfo, datetime_type, timedelta_from_seconds};
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyBytes, PyModule, PyStr, PyStrRef, PyTuple, PyType, PyTypeRef,
        },
        class::StaticType,
        function::{FuncArgs, IntoFuncArgs, KwArgs},
        protocol::{PyIter, PyIterReturn},
        types::{Constructor, Representable},
    };

    const EPOCHORDINAL: i64 = 719_163;
    const SOURCE_NOCACHE: u8 = 0;
    const SOURCE_CACHE: u8 = 1;
    const SOURCE_FILE: u8 = 2;
    const STRONG_CACHE_MAX: usize = 8;
    const DELTA_CACHE_MAX: usize = 512;
    const DAYS_IN_MONTH: [i32; 13] = [-1, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    const DAYS_BEFORE_MONTH: [i32; 13] =
        [-1, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    #[derive(Clone, Copy, Debug)]
    enum DateRule {
        Calendar {
            month: u8,
            week: u8,
            day: u8,
            hour: i32,
            minute: i32,
            second: i32,
        },
        Day {
            julian: bool,
            day: u16,
            hour: i32,
            minute: i32,
            second: i32,
        },
    }

    impl DateRule {
        fn timestamp(self, year: i32) -> i64 {
            match self {
                Self::Calendar {
                    month,
                    week,
                    day,
                    hour,
                    minute,
                    second,
                } => calendar_timestamp(year, month, week, day, hour, minute, second),
                Self::Day {
                    julian,
                    day,
                    hour,
                    minute,
                    second,
                } => day_timestamp(year, julian, day, hour, minute, second),
            }
        }
    }

    #[derive(Clone, Debug, Traverse)]
    struct TtInfo {
        utcoff: PyObjectRef,
        dstoff: PyObjectRef,
        tzname: PyObjectRef,
        #[pytraverse(skip)]
        utcoff_seconds: i64,
    }

    #[derive(Debug, Traverse)]
    struct TzRule {
        std: TtInfo,
        dst: Option<TtInfo>,
        #[pytraverse(skip)]
        std_only: bool,
        #[pytraverse(skip)]
        dst_diff: i64,
        #[pytraverse(skip)]
        start: Option<DateRule>,
        #[pytraverse(skip)]
        end: Option<DateRule>,
    }

    #[derive(Debug, Traverse)]
    struct ZoneData {
        #[pytraverse(skip)]
        trans_utc: Vec<i64>,
        // fold 0 keeps the larger offset, fold 1 the smaller.
        #[pytraverse(skip)]
        trans_wall: [Vec<i64>; 2],
        ttinfos: Vec<TtInfo>,
        #[pytraverse(skip)]
        trans_ttinfos: Vec<usize>,
        #[pytraverse(skip)]
        tti_before: Option<usize>,
        after: TzRule,
        #[pytraverse(skip)]
        fixed_offset: bool,
    }

    #[derive(Clone, Copy, Debug)]
    enum TtSel {
        Index(usize),
        Std,
        Dst,
        None,
    }

    enum TzFail {
        StdFormat,
        StdOffset,
        DstFormat,
        DstOffset,
        MissingRules,
        MalformedRule,
        Extra,
    }

    struct CacheState {
        strong: VecDeque<(PyObjectRef, PyObjectRef)>,
        weak: Option<PyObjectRef>,
        deltas: HashMap<i64, PyObjectRef>,
        delta_order: VecDeque<i64>,
    }

    fn empty_cache() -> CacheState {
        CacheState {
            strong: VecDeque::new(),
            weak: None,
            deltas: HashMap::new(),
            delta_order: VecDeque::new(),
        }
    }

    // `PyObjectRef` is `Send` only with threading, so a process-wide mutex is
    // not `Sync` otherwise. One interpreter thread shares a thread-local.
    #[cfg(feature = "threading")]
    fn cache() -> &'static Mutex<CacheState> {
        static CACHE: OnceLock<Mutex<CacheState>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(empty_cache()))
    }

    #[cfg(not(feature = "threading"))]
    thread_local! {
        static CACHE: RefCell<CacheState> = RefCell::new(empty_cache());
    }

    fn with_cache<R>(f: impl FnOnce(&mut CacheState) -> R) -> R {
        cfg_select! {
            feature = "threading" => {
                let mut cache = cache().lock().unwrap_or_else(|err| err.into_inner());
                f(&mut cache)
            }
            _ => { CACHE.with(|cache| f(&mut cache.borrow_mut())) }
        }
    }

    fn is_base(cls: &Py<PyType>) -> bool {
        cls.is(ZoneInfo::static_type())
    }

    fn strong_entries() -> Vec<(PyObjectRef, PyObjectRef)> {
        with_cache(|cache| cache.strong.iter().cloned().collect())
    }

    fn move_strong_to_front(zone: &PyObject) {
        with_cache(|cache| {
            let Some(pos) = cache.strong.iter().position(|(_, cached)| cached.is(zone)) else {
                return;
            };
            if pos != 0
                && let Some(entry) = cache.strong.remove(pos)
            {
                cache.strong.push_front(entry);
            }
        });
    }

    fn strong_cache_get(
        cls: &Py<PyType>,
        key: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if !is_base(cls) {
            return Ok(None);
        }
        for (cached_key, zone) in strong_entries() {
            if vm.bool_eq(key, &cached_key)? {
                move_strong_to_front(&zone);
                return Ok(Some(zone));
            }
        }
        Ok(None)
    }

    fn strong_cache_put(cls: &Py<PyType>, key: PyObjectRef, zone: PyObjectRef) {
        if !is_base(cls) {
            return;
        }
        with_cache(|cache| {
            cache.strong.push_front((key, zone));
            while cache.strong.len() > STRONG_CACHE_MAX {
                cache.strong.pop_back();
            }
        });
    }

    fn strong_cache_eject(cls: &Py<PyType>, key: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if !is_base(cls) {
            return Ok(());
        }
        for (cached_key, zone) in strong_entries() {
            if vm.bool_eq(key, &cached_key)? {
                with_cache(|cache| {
                    if let Some(pos) = cache.strong.iter().position(|(_, cached)| cached.is(&zone))
                    {
                        cache.strong.remove(pos);
                    }
                });
                return Ok(());
            }
        }
        Ok(())
    }

    fn strong_cache_clear(cls: &Py<PyType>) {
        if is_base(cls) {
            with_cache(|cache| cache.strong.clear());
        }
    }

    fn cached_delta(seconds: i64, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if let Some(delta) = with_cache(|cache| cache.deltas.get(&seconds).cloned()) {
            return Ok(delta);
        }
        let delta = timedelta_from_seconds(seconds, vm)?;
        Ok(with_cache(|cache| {
            if let Some(existing) = cache.deltas.get(&seconds) {
                return existing.clone();
            }
            if cache.deltas.len() >= DELTA_CACHE_MAX
                && let Some(old) = cache.delta_order.pop_front()
            {
                cache.deltas.remove(&old);
            }
            cache.deltas.insert(seconds, delta.clone());
            cache.delta_order.push_back(seconds);
            delta
        }))
    }

    fn new_weak_cache(vm: &VirtualMachine) -> PyResult {
        let weakref = vm.import("weakref", 0)?;
        let dict = weakref.get_attr("WeakValueDictionary", vm)?;
        dict.call((), vm)
    }

    fn base_weak_cache(vm: &VirtualMachine) -> PyResult {
        if let Some(weak) = with_cache(|cache| cache.weak.clone()) {
            return Ok(weak);
        }
        let weak = new_weak_cache(vm)?;
        Ok(with_cache(|cache| {
            if let Some(existing) = &cache.weak {
                return existing.clone();
            }
            cache.weak = Some(weak.clone());
            weak
        }))
    }

    fn get_weak_cache(cls: &Py<PyType>, vm: &VirtualMachine) -> PyResult {
        if is_base(cls) {
            base_weak_cache(vm)
        } else {
            cls.as_object().get_attr("_weak_cache", vm)
        }
    }

    fn call_attr(
        module: &'static str,
        name: &'static str,
        args: impl IntoFuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        // import() returns the top-level package for a dotted name.
        let imported = vm.import(module, 0)?;
        let module = match module.rsplit_once('.') {
            Some((_, attr)) => imported.get_attr(attr, vm)?,
            None => imported,
        };
        let func = module.get_attr(name, vm)?;
        func.call(args, vm)
    }

    fn floor_mod(value: i64, modulo: i64) -> i64 {
        let mut out = value % modulo;
        if out < 0 {
            out += modulo;
        }
        out
    }

    fn is_leap_year(year: i32) -> bool {
        let year = year as u32;
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    }

    fn ymd_to_ord(y: i32, m: i32, d: i32) -> i32 {
        let y = y - 1;
        let days_before_year = y * 365 + y / 4 - y / 100 + y / 400;
        let mut yearday = DAYS_BEFORE_MONTH[m as usize];
        if m > 2 && is_leap_year(y + 1) {
            yearday += 1;
        }
        days_before_year + yearday + d
    }

    fn calendar_timestamp(
        year: i32,
        month: u8,
        week: u8,
        day: u8,
        hour: i32,
        minute: i32,
        second: i32,
    ) -> i64 {
        let month = i32::from(month);
        let first_day = floor_mod(i64::from(ymd_to_ord(year, month, 1)) + 6, 7);
        let mut days_in_month = DAYS_IN_MONTH[month as usize];
        if month == 2 && is_leap_year(year) {
            days_in_month += 1;
        }
        let mut month_day = floor_mod(i64::from(day) - (first_day + 1), 7) + 1;
        month_day += (i64::from(week) - 1) * 7;
        if month_day > i64::from(days_in_month) {
            month_day -= 7;
        }
        let ordinal = i64::from(ymd_to_ord(year, month, month_day as i32)) - EPOCHORDINAL;
        ordinal * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second)
    }

    fn day_timestamp(
        year: i32,
        julian: bool,
        day: u16,
        hour: i32,
        minute: i32,
        second: i32,
    ) -> i64 {
        let days_before_year = i64::from(ymd_to_ord(year, 1, 1)) - EPOCHORDINAL - 1;
        let mut day = i64::from(day);
        if julian && day >= 59 && is_leap_year(year) {
            day += 1;
        }
        (days_before_year + day) * 86_400
            + i64::from(hour) * 3_600
            + i64::from(minute) * 60
            + i64::from(second)
    }

    fn bisect_right(value: i64, arr: &[i64]) -> usize {
        arr.partition_point(|&item| item <= value)
    }

    fn order_offsets(a: i64, b: i64) -> (i64, i64) {
        if b > a { (b, a) } else { (a, b) }
    }

    fn utcoff_to_dstoff(trans_idx: &[usize], utcoffs: &[i64], isdsts: &[bool]) -> Vec<i64> {
        let num_ttinfos = utcoffs.len();
        let num_transitions = trans_idx.len();
        let mut dstoffs = vec![0_i64; num_ttinfos];
        let dst_count = num_ttinfos;
        let mut dst_found = 0_usize;
        for i in 1..num_transitions {
            if dst_count == dst_found {
                break;
            }
            let idx = trans_idx[i];
            let mut comp_idx = trans_idx[i - 1];
            if !isdsts[idx] || dstoffs[idx] != 0 {
                continue;
            }
            let utcoff = utcoffs[idx];
            let mut dstoff = if !isdsts[comp_idx] {
                utcoff - utcoffs[comp_idx]
            } else {
                0
            };
            if dstoff == 0 && idx + 1 < num_ttinfos && i + 1 < num_transitions {
                comp_idx = trans_idx[i + 1];
                if isdsts[comp_idx] {
                    continue;
                }
                dstoff = utcoff - utcoffs[comp_idx];
            }
            if dstoff != 0 {
                dst_found += 1;
                dstoffs[idx] = dstoff;
            }
        }
        if dst_found < dst_count {
            for idx in 0..num_ttinfos {
                if isdsts[idx] && dstoffs[idx] == 0 {
                    dstoffs[idx] = 3_600;
                }
            }
        }
        dstoffs
    }

    fn ts_to_local(trans_idx: &[usize], trans_utc: &[i64], utcoff: &[i64]) -> [Vec<i64>; 2] {
        let n = trans_utc.len();
        if n == 0 {
            return [Vec::new(), Vec::new()];
        }
        let mut wall0 = trans_utc.to_vec();
        let mut wall1 = trans_utc.to_vec();
        let (offset_0, offset_1) = if utcoff.len() > 1 {
            order_offsets(utcoff[0], utcoff[trans_idx[0]])
        } else {
            (utcoff[0], utcoff[0])
        };
        wall0[0] += offset_0;
        wall1[0] += offset_1;
        for i in 1..n {
            let (off0, off1) = order_offsets(utcoff[trans_idx[i - 1]], utcoff[trans_idx[i]]);
            wall0[i] += off0;
            wall1[i] += off1;
        }
        [wall0, wall1]
    }

    struct Parser<'a> {
        bytes: &'a [u8],
        i: usize,
    }

    impl Parser<'_> {
        fn peek(&self) -> Option<u8> {
            match self.bytes.get(self.i).copied() {
                Some(0) | None => None,
                Some(byte) => Some(byte),
            }
        }

        fn bump(&mut self) -> Option<u8> {
            let byte = self.peek()?;
            self.i += 1;
            Some(byte)
        }
    }

    fn parse_digits(parser: &mut Parser<'_>, min: usize, max: usize) -> Option<i32> {
        let mut value = 0_i32;
        let mut count = 0_usize;
        while count < max {
            let Some(byte) = parser.peek() else {
                return (count >= min).then_some(value);
            };
            if !byte.is_ascii_digit() {
                return (count >= min).then_some(value);
            }
            parser.bump();
            value = value * 10 + i32::from(byte - b'0');
            count += 1;
        }
        Some(value)
    }

    fn parse_abbr(parser: &mut Parser<'_>) -> Option<String> {
        if parser.peek() == Some(b'<') {
            parser.bump();
            let start = parser.i;
            loop {
                let byte = parser.peek()?;
                if byte == b'>' {
                    break;
                }
                if !(byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'-') {
                    return None;
                }
                parser.bump();
            }
            let end = parser.i;
            parser.bump();
            return Some(String::from_utf8_lossy(&parser.bytes[start..end]).into_owned());
        }
        let start = parser.i;
        while parser.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
            parser.bump();
        }
        if parser.i == start {
            return None;
        }
        Some(String::from_utf8_lossy(&parser.bytes[start..parser.i]).into_owned())
    }

    fn parse_transition_time(parser: &mut Parser<'_>) -> Option<(i32, i32, i32)> {
        let mut sign = 1_i32;
        if matches!(parser.peek(), Some(b'-' | b'+')) {
            if parser.peek() == Some(b'-') {
                sign = -1;
            }
            parser.bump();
        }
        let hour = parse_digits(parser, 1, 3)? * sign;
        let mut minute = 0_i32;
        let mut second = 0_i32;
        if parser.peek() == Some(b':') {
            parser.bump();
            minute = parse_digits(parser, 2, 2)? * sign;
            if parser.peek() == Some(b':') {
                parser.bump();
                second = parse_digits(parser, 2, 2)? * sign;
            }
        }
        Some((hour, minute, second))
    }

    fn parse_tz_delta(parser: &mut Parser<'_>) -> Option<i64> {
        let (hours, minutes, seconds) = parse_transition_time(parser)?;
        if !(-24..=24).contains(&hours) {
            return None;
        }
        Some(-((i64::from(hours) * 3_600) + (i64::from(minutes) * 60) + i64::from(seconds)))
    }

    fn calendar_rule(
        month: i32,
        week: i32,
        day: i32,
        hour: i32,
        minute: i32,
        second: i32,
    ) -> Option<DateRule> {
        if !(1..=12).contains(&month)
            || !(1..=5).contains(&week)
            || !(0..=6).contains(&day)
            || !(-167..=167).contains(&hour)
        {
            return None;
        }
        Some(DateRule::Calendar {
            month: month as u8,
            week: week as u8,
            day: day as u8,
            hour,
            minute,
            second,
        })
    }

    fn day_rule(julian: bool, day: i32, hour: i32, minute: i32, second: i32) -> Option<DateRule> {
        let min_day = i32::from(julian);
        if day < min_day || day > 365 || !(-167..=167).contains(&hour) {
            return None;
        }
        Some(DateRule::Day {
            julian,
            day: day as u16,
            hour,
            minute,
            second,
        })
    }

    fn parse_transition_rule(parser: &mut Parser<'_>) -> Option<DateRule> {
        let mut hour = 2_i32;
        let mut minute = 0_i32;
        let mut second = 0_i32;
        if parser.peek() == Some(b'M') {
            parser.bump();
            let month = parse_digits(parser, 1, 2)?;
            if parser.bump() != Some(b'.') {
                return None;
            }
            let week = parse_digits(parser, 1, 1)?;
            if parser.bump() != Some(b'.') {
                return None;
            }
            let day = parse_digits(parser, 1, 1)?;
            if parser.peek() == Some(b'/') {
                parser.bump();
                (hour, minute, second) = parse_transition_time(parser)?;
            }
            return calendar_rule(month, week, day, hour, minute, second);
        }
        let mut julian = false;
        if parser.peek() == Some(b'J') {
            julian = true;
            parser.bump();
        }
        let day = parse_digits(parser, 1, 3)?;
        if parser.peek() == Some(b'/') {
            parser.bump();
            (hour, minute, second) = parse_transition_time(parser)?;
        }
        day_rule(julian, day, hour, minute, second)
    }

    fn tz_error(kind: TzFail, shown: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let message = match kind {
            TzFail::StdFormat => format!("Invalid STD format in {shown}"),
            TzFail::StdOffset => format!("Invalid STD offset in {shown}"),
            TzFail::DstFormat => format!("Invalid DST format in {shown}"),
            TzFail::DstOffset => format!("Invalid DST offset in {shown}"),
            TzFail::MissingRules => format!("Missing transition rules in TZ string: {shown}"),
            TzFail::MalformedRule => format!("Malformed transition rule in TZ string: {shown}"),
            TzFail::Extra => format!("Extraneous characters at end of TZ string: {shown}"),
        };
        vm.new_value_error(message)
    }

    fn build_ttinfo(
        utcoff_seconds: i64,
        dst_seconds: i64,
        tzname: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<TtInfo> {
        Ok(TtInfo {
            utcoff: cached_delta(utcoff_seconds, vm)?,
            dstoff: cached_delta(dst_seconds, vm)?,
            tzname,
            utcoff_seconds,
        })
    }

    fn build_tzrule(
        std_abbr: PyObjectRef,
        dst_abbr: Option<PyObjectRef>,
        std_offset: i64,
        dst_offset: i64,
        start: Option<DateRule>,
        end: Option<DateRule>,
        vm: &VirtualMachine,
    ) -> PyResult<TzRule> {
        let std = build_ttinfo(std_offset, 0, std_abbr, vm)?;
        let (dst, std_only, dst_diff) = if let Some(dst_abbr) = dst_abbr {
            let dst_diff = dst_offset - std_offset;
            let dst = build_ttinfo(dst_offset, dst_diff, dst_abbr, vm)?;
            (Some(dst), false, dst_diff)
        } else {
            (None, true, 0)
        };
        Ok(TzRule {
            std,
            dst,
            std_only,
            dst_diff,
            start,
            end,
        })
    }

    fn parse_tz_str(bytes: &[u8], shown: &str, vm: &VirtualMachine) -> PyResult<TzRule> {
        let mut parser = Parser { bytes, i: 0 };
        let std_abbr =
            parse_abbr(&mut parser).ok_or_else(|| tz_error(TzFail::StdFormat, shown, vm))?;
        let std_offset =
            parse_tz_delta(&mut parser).ok_or_else(|| tz_error(TzFail::StdOffset, shown, vm))?;
        if parser.peek().is_none() {
            return build_tzrule(
                vm.ctx.new_str(std_abbr).into(),
                None,
                std_offset,
                0,
                None,
                None,
                vm,
            );
        }
        let dst_abbr =
            parse_abbr(&mut parser).ok_or_else(|| tz_error(TzFail::DstFormat, shown, vm))?;
        let dst_offset = if parser.peek() == Some(b',') {
            std_offset + 3_600
        } else {
            parse_tz_delta(&mut parser).ok_or_else(|| tz_error(TzFail::DstOffset, shown, vm))?
        };
        let mut rules = [None, None];
        for rule in &mut rules {
            if parser.peek() != Some(b',') {
                return Err(tz_error(TzFail::MissingRules, shown, vm));
            }
            parser.bump();
            *rule = Some(
                parse_transition_rule(&mut parser)
                    .ok_or_else(|| tz_error(TzFail::MalformedRule, shown, vm))?,
            );
        }
        if parser.peek().is_some() {
            return Err(tz_error(TzFail::Extra, shown, vm));
        }
        build_tzrule(
            vm.ctx.new_str(std_abbr).into(),
            Some(vm.ctx.new_str(dst_abbr).into()),
            std_offset,
            dst_offset,
            rules[0],
            rules[1],
            vm,
        )
    }

    fn tuple_owned(tup: &Py<PyTuple>, index: usize, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        tup.as_slice()
            .get(index)
            .cloned()
            .ok_or_else(|| vm.new_index_error("tuple index out of range"))
    }

    fn tuple_i64(tup: &Py<PyTuple>, index: usize, vm: &VirtualMachine) -> PyResult<i64> {
        tuple_owned(tup, index, vm)?.try_to_value(vm)
    }

    fn as_exact_tuple<'a>(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<&'a Py<PyTuple>> {
        if let Some(tup) = obj.downcast_ref_if_exact::<PyTuple>(vm) {
            return Ok(tup);
        }
        let repr = obj.repr(vm)?;
        Err(vm.new_type_error(format!(
            "Invalid data result type: {}",
            repr.to_string_lossy()
        )))
    }

    fn as_tuple<'a>(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<&'a Py<PyTuple>> {
        obj.downcast_ref::<PyTuple>().ok_or_else(|| {
            vm.new_type_error(format!("expected tuple, got '{}'", obj.class().slot_name()))
        })
    }

    fn transition_index(value: i64, num_ttinfos: usize, vm: &VirtualMachine) -> PyResult<usize> {
        let Ok(index) = usize::try_from(value) else {
            return Err(vm.new_value_error(format!(
                "Invalid transition index found while reading TZif: {value}"
            )));
        };
        if index >= num_ttinfos {
            return Err(vm.new_value_error(format!(
                "Invalid transition index found while reading TZif: {value}"
            )));
        }
        Ok(index)
    }

    fn read_transitions(
        trans_idx_list: &Py<PyTuple>,
        trans_utc_list: &Py<PyTuple>,
        num_ttinfos: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<usize>, Vec<i64>)> {
        let n = trans_utc_list.as_slice().len();
        let mut trans_idx = Vec::with_capacity(n);
        let mut trans_utc = Vec::with_capacity(n);
        for i in 0..n {
            trans_utc.push(tuple_i64(trans_utc_list, i, vm)?);
            let raw = tuple_i64(trans_idx_list, i, vm)?;
            trans_idx.push(transition_index(raw, num_ttinfos, vm)?);
        }
        Ok((trans_idx, trans_utc))
    }

    fn read_offsets(
        utcoff_list: &Py<PyTuple>,
        isdst_list: &Py<PyTuple>,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<i64>, Vec<bool>)> {
        let n = utcoff_list.as_slice().len();
        let mut utcoffs = Vec::with_capacity(n);
        let mut isdsts = Vec::with_capacity(n);
        for i in 0..n {
            utcoffs.push(tuple_i64(utcoff_list, i, vm)?);
            isdsts.push(tuple_owned(isdst_list, i, vm)?.try_to_bool(vm)?);
        }
        Ok((utcoffs, isdsts))
    }

    fn ttinfo_eq(left: &TtInfo, right: &TtInfo, vm: &VirtualMachine) -> PyResult<bool> {
        if !vm.bool_eq(&left.utcoff, &right.utcoff)? {
            return Ok(false);
        }
        if !vm.bool_eq(&left.dstoff, &right.dstoff)? {
            return Ok(false);
        }
        vm.bool_eq(&left.tzname, &right.tzname)
    }

    fn rule_from_ttinfo(tti: &TtInfo, vm: &VirtualMachine) -> PyResult<TzRule> {
        let mut rule = build_tzrule(
            tti.tzname.clone(),
            None,
            tti.utcoff_seconds,
            0,
            None,
            None,
            vm,
        )?;
        if tti.dstoff.try_to_bool(vm)? {
            rule.std.dstoff = tti.dstoff.clone();
        }
        Ok(rule)
    }

    fn load_data(file: &PyObject, vm: &VirtualMachine) -> PyResult<ZoneData> {
        let data = call_attr("zoneinfo._common", "load_data", (file.to_owned(),), vm)?;
        let data_tuple = as_exact_tuple(&data, vm)?;
        let trans_idx_obj = tuple_owned(data_tuple, 0, vm)?;
        let trans_utc_obj = tuple_owned(data_tuple, 1, vm)?;
        let utcoff_obj = tuple_owned(data_tuple, 2, vm)?;
        let isdst_obj = tuple_owned(data_tuple, 3, vm)?;
        let abbr_obj = tuple_owned(data_tuple, 4, vm)?;
        let tz_str = tuple_owned(data_tuple, 5, vm)?;
        let trans_idx_list = as_tuple(&trans_idx_obj, vm)?;
        let trans_utc_list = as_tuple(&trans_utc_obj, vm)?;
        let utcoff_list = as_tuple(&utcoff_obj, vm)?;
        let isdst_list = as_tuple(&isdst_obj, vm)?;
        let abbr = as_tuple(&abbr_obj, vm)?;

        let num_ttinfos = utcoff_list.as_slice().len();
        let (trans_idx, trans_utc) =
            read_transitions(trans_idx_list, trans_utc_list, num_ttinfos, vm)?;
        let (utcoffs, isdsts) = read_offsets(utcoff_list, isdst_list, vm)?;
        let dstoffs = utcoff_to_dstoff(&trans_idx, &utcoffs, &isdsts);
        let trans_wall = ts_to_local(&trans_idx, &trans_utc, &utcoffs);

        let mut ttinfos = Vec::with_capacity(num_ttinfos);
        for i in 0..num_ttinfos {
            let tzname = tuple_owned(abbr, i, vm)?;
            ttinfos.push(build_ttinfo(utcoffs[i], dstoffs[i], tzname, vm)?);
        }
        let trans_ttinfos = trans_idx.clone();
        let mut tti_before = isdsts.iter().position(|isdst| !isdst);
        if tti_before.is_none() && !ttinfos.is_empty() {
            tti_before = Some(0);
        }

        let use_tz = !vm.is_none(&tz_str) && tz_str.try_to_bool(vm)?;
        let after = if use_tz {
            let bytes = tz_str.downcast_ref::<PyBytes>().ok_or_else(|| {
                vm.new_type_error(format!(
                    "expected bytes, {} found",
                    tz_str.class().slot_name()
                ))
            })?;
            let shown = tz_str.repr(vm)?.to_string_lossy().into_owned();
            parse_tz_str(bytes.as_bytes(), &shown, vm)?
        } else if ttinfos.is_empty() {
            return Err(vm.new_value_error("No time zone information found."));
        } else {
            let idx = if trans_idx.is_empty() {
                ttinfos.len() - 1
            } else {
                trans_idx[trans_idx.len() - 1]
            };
            rule_from_ttinfo(&ttinfos[idx], vm)?
        };

        let fixed_offset = if ttinfos.len() > 1 || !after.std_only {
            false
        } else if ttinfos.is_empty() {
            true
        } else {
            ttinfo_eq(&ttinfos[0], &after.std, vm)?
        };

        Ok(ZoneData {
            trans_utc,
            trans_wall,
            ttinfos,
            trans_ttinfos,
            tti_before,
            after,
            fixed_offset,
        })
    }

    fn open_zone(key: &PyObject, vm: &VirtualMachine) -> PyResult {
        let path = call_attr("zoneinfo._tzpath", "find_tzfile", (key.to_owned(),), vm)?;
        if vm.is_none(&path) {
            return call_attr("zoneinfo._common", "load_tzdata", (key.to_owned(),), vm);
        }
        let io = vm.import("io", 0)?;
        let open = io.get_attr("open", vm)?;
        open.call((path, vm.ctx.new_str("rb")), vm)
    }

    fn close_zone(
        file: &PyObject,
        load: PyResult<ZoneData>,
        vm: &VirtualMachine,
    ) -> PyResult<ZoneData> {
        match load {
            Ok(data) => {
                vm.call_method(file, "close", ())?;
                Ok(data)
            }
            Err(load_err) => match vm.call_method(file, "close", ()) {
                Ok(_) => Err(load_err),
                Err(close_err) => {
                    close_err.set_context(Some(load_err));
                    Err(close_err)
                }
            },
        }
    }

    fn new_instance(
        cls: PyTypeRef,
        key: PyObjectRef,
        source: u8,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<ZoneInfo>> {
        let file = open_zone(&key, vm)?;
        let data = close_zone(&file, load_data(&file, vm), vm)?;
        ZoneInfo {
            base: PyTzInfo::default(),
            key,
            file_repr: None,
            source,
            data,
        }
        .into_ref_with_type(vm, cls)
    }

    fn cache_mismatch(
        instance: &PyObject,
        cls: &Py<PyType>,
        key: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult {
        let qual = instance.class().fully_qualified_name(vm)?;
        let key_repr = key.repr(vm)?.to_string_lossy().into_owned();
        Err(vm.new_runtime_error(format!(
            "Unexpected instance of {qual} in {} weak cache for key {key_repr}",
            cls.name()
        )))
    }

    fn cached_new(cls: PyTypeRef, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(hit) = strong_cache_get(&cls, &key, vm)? {
            return Ok(hit);
        }
        let weak = get_weak_cache(&cls, vm)?;
        let found = vm.call_method(&weak, "get", (key.clone(), vm.ctx.none()))?;
        let instance = if vm.is_none(&found) {
            let fresh = new_instance(cls.clone(), key.clone(), SOURCE_CACHE, vm)?;
            let fresh: PyObjectRef = fresh.into();
            vm.call_method(&weak, "setdefault", (key.clone(), fresh))?
        } else {
            found
        };
        if !instance.fast_isinstance(&cls) {
            return cache_mismatch(&instance, &cls, &key, vm);
        }
        strong_cache_put(&cls, key, instance.clone());
        Ok(instance)
    }

    fn int_attr(obj: &PyObject, name: &'static str, vm: &VirtualMachine) -> PyResult<i64> {
        obj.get_attr(name, vm)?.try_to_value(vm)
    }

    fn local_timestamp(dt: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
        let ordinal: i64 = vm.call_method(dt, "toordinal", ())?.try_to_value(vm)?;
        let hour = int_attr(dt, "hour", vm)?;
        let minute = int_attr(dt, "minute", vm)?;
        let second = int_attr(dt, "second", vm)?;
        Ok((ordinal - EPOCHORDINAL) * 86_400 + hour * 3_600 + minute * 60 + second)
    }

    fn fold_index(dt: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(usize::from(int_attr(dt, "fold", vm)? != 0))
    }

    fn ttinfo(data: &ZoneData, sel: TtSel) -> Option<&TtInfo> {
        match sel {
            TtSel::Index(index) => data.ttinfos.get(index),
            TtSel::Std => Some(&data.after.std),
            TtSel::Dst => data.after.dst.as_ref(),
            TtSel::None => None,
        }
    }

    fn utcoff_seconds(data: &ZoneData, sel: TtSel) -> i64 {
        ttinfo(data, sel).map_or(0, |tti| tti.utcoff_seconds)
    }

    fn component(data: &ZoneData, sel: TtSel, which: u8, vm: &VirtualMachine) -> PyObjectRef {
        let Some(tti) = ttinfo(data, sel) else {
            return vm.ctx.none();
        };
        match which {
            0 => tti.utcoff.clone(),
            1 => tti.dstoff.clone(),
            _ => tti.tzname.clone(),
        }
    }

    fn rule_bounds(rule: &TzRule, year: i32) -> Option<(i64, i64)> {
        Some((rule.start?.timestamp(year), rule.end?.timestamp(year)))
    }

    fn find_tzrule_ttinfo(rule: &TzRule, ts: i64, fold: bool, year: i32) -> TtSel {
        if rule.std_only {
            return TtSel::Std;
        }
        let Some((mut start, mut end)) = rule_bounds(rule, year) else {
            return TtSel::Std;
        };
        if fold == (rule.dst_diff >= 0) {
            end -= rule.dst_diff;
        } else {
            start += rule.dst_diff;
        }
        let isdst = if start < end {
            ts >= start && ts < end
        } else {
            ts < end || ts >= start
        };
        if isdst { TtSel::Dst } else { TtSel::Std }
    }

    fn find_tzrule_fromutc(rule: &TzRule, ts: i64, year: i32) -> (TtSel, bool) {
        if rule.std_only {
            return (TtSel::Std, false);
        }
        let Some((mut start, mut end)) = rule_bounds(rule, year) else {
            return (TtSel::Std, false);
        };
        start -= rule.std.utcoff_seconds;
        let Some(dst) = &rule.dst else {
            return (TtSel::Std, false);
        };
        end -= dst.utcoff_seconds;
        let isdst = if start < end {
            ts >= start && ts < end
        } else {
            ts < end || ts >= start
        };
        let (ambig_start, ambig_end) = if rule.dst_diff > 0 {
            (end, end + rule.dst_diff)
        } else {
            (start, start - rule.dst_diff)
        };
        let fold = ts >= ambig_start && ts < ambig_end;
        let sel = if isdst { TtSel::Dst } else { TtSel::Std };
        (sel, fold)
    }

    fn before_sel(data: &ZoneData) -> TtSel {
        match data.tti_before {
            Some(index) => TtSel::Index(index),
            None => TtSel::None,
        }
    }

    fn find_ttinfo(data: &ZoneData, dt: &PyObject, vm: &VirtualMachine) -> PyResult<TtSel> {
        if vm.is_none(dt) {
            return Ok(if data.fixed_offset {
                TtSel::Std
            } else {
                TtSel::None
            });
        }
        let ts = local_timestamp(dt, vm)?;
        let fold = fold_index(dt, vm)?;
        let num_trans = data.trans_utc.len();
        let wall = &data.trans_wall[fold];
        if num_trans > 0 && ts < wall[0] {
            return Ok(before_sel(data));
        }
        if num_trans == 0 || ts > wall[num_trans - 1] {
            let year = int_attr(dt, "year", vm)? as i32;
            return Ok(find_tzrule_ttinfo(&data.after, ts, fold != 0, year));
        }
        let idx = bisect_right(ts, wall) - 1;
        Ok(TtSel::Index(data.trans_ttinfos[idx]))
    }

    fn pickling_error(vm: &VirtualMachine) -> PyResult<PyBaseExceptionRef> {
        let pickle = vm.import("pickle", 0)?;
        let ty = pickle
            .get_attr("PicklingError", vm)?
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("pickle.PicklingError is not a type"))?;
        Ok(vm.new_exception_msg(
            ty,
            "Cannot pickle a ZoneInfo file from a file stream.".into(),
        ))
    }

    #[derive(FromArgs)]
    struct NewArgs {
        #[pyarg(any)]
        key: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct NoCacheArgs {
        #[pyarg(any)]
        key: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct FromFileArgs {
        #[pyarg(positional)]
        file_obj: PyObjectRef,
        #[pyarg(any, optional)]
        key: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct ClearCacheArgs {
        #[pyarg(named, optional)]
        only_keys: Option<PyObjectRef>,
    }

    #[pyattr]
    #[pyclass(module = "zoneinfo", name = "ZoneInfo", base = PyTzInfo, traverse)]
    #[derive(Debug)]
    struct ZoneInfo {
        #[pytraverse(skip)]
        base: PyTzInfo,
        #[pymember(type = "object_ex")]
        key: PyObjectRef,
        #[pytraverse(skip)]
        file_repr: Option<String>,
        #[pytraverse(skip)]
        source: u8,
        data: ZoneData,
    }

    impl Constructor for ZoneInfo {
        type Args = NewArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: NewArgs = args.bind(vm)?;
            cached_new(cls, args.key, vm)
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unreachable!("slot_new is overridden")
        }
    }

    impl Representable for ZoneInfo {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let name = zelf.class().fully_qualified_name(vm)?;
            if vm.is_none(&zelf.key) {
                let file = zelf.file_repr.as_deref().unwrap_or("");
                return Ok(format!("{name}.from_file({file})"));
            }
            let key = zelf.key.repr(vm)?.to_string_lossy().into_owned();
            Ok(format!("{name}(key={key})"))
        }
    }

    #[pyclass(
        with(Constructor, Representable),
        flags(BASETYPE, HAS_WEAKREF, IMMUTABLETYPE, HEAPTYPE)
    )]
    impl ZoneInfo {
        #[pyclassmethod]
        fn clear_cache(cls: PyTypeRef, args: ClearCacheArgs, vm: &VirtualMachine) -> PyResult<()> {
            let weak = get_weak_cache(&cls, vm)?;
            let Some(only_keys) = args.only_keys else {
                let cleared = vm.call_method(&weak, "clear", ());
                strong_cache_clear(&cls);
                cleared?;
                return Ok(());
            };
            let iter = PyIter::try_from_object(vm, only_keys)?;
            loop {
                match iter.next(vm)? {
                    PyIterReturn::StopIteration(_) => break,
                    PyIterReturn::Return(item) => {
                        strong_cache_eject(&cls, &item, vm)?;
                        vm.call_method(&weak, "pop", (item, vm.ctx.none()))?;
                    }
                }
            }
            Ok(())
        }

        #[pyclassmethod]
        fn no_cache(
            cls: PyTypeRef,
            args: NoCacheArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            new_instance(cls, args.key, SOURCE_NOCACHE, vm)
        }

        #[pyclassmethod]
        fn from_file(
            cls: PyTypeRef,
            args: FromFileArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let file_repr = args.file_obj.repr(vm)?.to_string_lossy().into_owned();
            let key = args.key.unwrap_or_else(|| vm.ctx.none());
            let data = load_data(&args.file_obj, vm)?;
            Self {
                base: PyTzInfo::default(),
                key,
                file_repr: Some(file_repr),
                source: SOURCE_FILE,
                data,
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn utcoffset(&self, dt: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let sel = find_ttinfo(&self.data, &dt, vm)?;
            Ok(component(&self.data, sel, 0, vm))
        }

        #[pymethod]
        fn dst(&self, dt: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let sel = find_ttinfo(&self.data, &dt, vm)?;
            Ok(component(&self.data, sel, 1, vm))
        }

        #[pymethod]
        fn tzname(&self, dt: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let sel = find_ttinfo(&self.data, &dt, vm)?;
            Ok(component(&self.data, sel, 2, vm))
        }

        #[pymethod]
        fn fromutc(zelf: &Py<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            if !object.fast_isinstance(datetime_type()) {
                return Err(vm.new_type_error("fromutc: argument must be a datetime"));
            }
            let tzinfo = object.get_attr("tzinfo", vm)?;
            if !tzinfo.is(zelf) {
                return Err(vm.new_value_error("fromutc: dt.tzinfo is not self"));
            }
            let ts = local_timestamp(&object, vm)?;
            let year = int_attr(&object, "year", vm)? as i32;
            let data = &zelf.data;
            let num_trans = data.trans_utc.len();
            let (sel, fold) = if num_trans >= 1 && ts < data.trans_utc[0] {
                (before_sel(data), false)
            } else if num_trans == 0 || ts > data.trans_utc[num_trans - 1] {
                let (sel, mut fold) = find_tzrule_fromutc(&data.after, ts, year);
                if num_trans > 0 {
                    let prev = if num_trans == 1 {
                        before_sel(data)
                    } else {
                        TtSel::Index(data.trans_ttinfos[num_trans - 2])
                    };
                    let diff = utcoff_seconds(data, prev) - utcoff_seconds(data, sel);
                    if diff > 0 && ts < data.trans_utc[num_trans - 1] + diff {
                        fold = true;
                    }
                }
                (sel, fold)
            } else {
                let idx = bisect_right(ts, &data.trans_utc);
                let (prev, sel) = if idx >= 2 {
                    (
                        TtSel::Index(data.trans_ttinfos[idx - 2]),
                        TtSel::Index(data.trans_ttinfos[idx - 1]),
                    )
                } else {
                    (before_sel(data), TtSel::Index(data.trans_ttinfos[0]))
                };
                let shift = utcoff_seconds(data, prev) - utcoff_seconds(data, sel);
                let fold = shift > ts - data.trans_utc[idx - 1];
                (sel, fold)
            };
            let Some(tti) = ttinfo(data, sel) else {
                return Err(vm.new_value_error("No time zone information found."));
            };
            let utcoff = tti.utcoff.clone();
            let added = vm._add(&object, &utcoff)?;
            if !fold {
                return Ok(added);
            }
            let kwargs: KwArgs =
                core::iter::once(("fold".to_owned(), vm.ctx.new_int(1).into())).collect();
            vm.call_method(
                &added,
                "replace",
                FuncArgs::new(Vec::<PyObjectRef>::new(), kwargs),
            )
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            if zelf.source == SOURCE_FILE {
                return Err(pickling_error(vm)?);
            }
            let constructor = zelf.as_object().get_attr("_unpickle", vm)?;
            let from_cache = zelf.source == SOURCE_CACHE;
            Ok(vm
                .new_tuple((
                    constructor,
                    vm.new_tuple((zelf.key.clone(), vm.ctx.new_bool(from_cache))),
                ))
                .into())
        }

        #[pyclassmethod]
        fn _unpickle(
            cls: PyTypeRef,
            key: PyObjectRef,
            from_cache: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let from_cache = (from_cache.try_index(vm)?.as_u64_mask() as u8) != 0;
            if from_cache {
                cls.as_object().call((key,), vm)
            } else {
                Ok(new_instance(cls, key, SOURCE_NOCACHE, vm)?.into())
            }
        }

        #[pyclassmethod]
        fn __init_subclass__(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            if is_base(&cls) {
                return Ok(());
            }
            let cache = new_weak_cache(vm)?;
            cls.as_object().set_attr("_weak_cache", cache, vm)?;
            Ok(())
        }

        #[pyslot]
        fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let zelf = zelf.downcast_ref::<Self>().unwrap();
            if !vm.is_none(&zelf.key) {
                if let Some(key) = zelf.key.downcast_ref::<PyStr>() {
                    return Ok(key.to_owned());
                }
                return Err(vm.new_type_error(format!(
                    "__str__ returned non-string (type {})",
                    zelf.key.class().slot_name()
                )));
            }
            Ok(vm.ctx.new_str(Self::repr_str(zelf, vm)?))
        }
    }

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        // A blocked `_datetime` must fail this import so `zoneinfo` can fall back.
        vm.import("_datetime", 0)?;
        __module_exec(vm, module);
        Ok(())
    }
}
