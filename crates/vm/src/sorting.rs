// TODO: MERGESTATE_TEMP_SIZE unused — buf is a dynamic Vec, not a fixed stack array.
const MIN_GALLOP: usize = 7;
const MAX_MINRUN: usize = 64;

enum Breakout {
    Succeed,
    CopyA,
    CopyB,
}

#[derive(Clone, Copy)]
struct Run {
    base: usize,
    len: usize,
    power: u32,
}

struct MergeState<T> {
    buf: Vec<T>,
    min_gallop: usize,
    pending: Vec<Run>,
}

impl<T: Clone> MergeState<T> {
    fn merge_lo<E, F>(
        &mut self,
        values: &mut [T],
        is_lt: &mut F,
        start_a: usize,
        mut len_a: usize,
        start_b: usize,
        mut len_b: usize,
    ) -> Result<(), E>
    where
        F: FnMut(&T, &T) -> Result<bool, E>,
    {
        debug_assert!(len_a > 0);
        debug_assert!(len_b > 0);
        debug_assert!(start_a + len_a == start_b);

        self.buf.clear();
        self.buf
            .extend_from_slice(&values[start_a..start_a + len_a]);

        let mut cursor_a = 0;
        let mut cursor_b = start_b;
        let mut dest = start_a;

        values[dest] = values[cursor_b].clone();
        dest += 1;
        cursor_b += 1;
        len_b -= 1;

        if len_b == 0 {
            values[dest..dest + len_a].clone_from_slice(&self.buf[cursor_a..cursor_a + len_a]);
            return Ok(());
        }
        if len_a == 1 {
            copy_within_clone(values, cursor_b, dest, len_b);
            values[dest + len_b] = self.buf[cursor_a].clone();
            return Ok(());
        }

        let mut min_gallop = self.min_gallop;
        let mut breakout: Option<Breakout> = None;

        loop {
            let mut a_count = 0;
            let mut b_count = 0;

            loop {
                if is_lt(&values[cursor_b], &self.buf[cursor_a])? {
                    values[dest] = values[cursor_b].clone();
                    dest += 1;
                    cursor_b += 1;
                    len_b -= 1;
                    b_count += 1;
                    a_count = 0;
                    if len_b == 0 {
                        breakout = Some(Breakout::Succeed);
                        break;
                    }
                    if b_count >= min_gallop {
                        break;
                    }
                } else {
                    values[dest] = self.buf[cursor_a].clone();
                    dest += 1;
                    cursor_a += 1;
                    len_a -= 1;
                    a_count += 1;
                    b_count = 0;
                    if len_a == 1 {
                        breakout = Some(Breakout::CopyB);
                        break;
                    }
                    if a_count >= min_gallop {
                        break;
                    }
                }
            }

            if breakout.is_some() {
                break;
            }

            min_gallop += 1;
            loop {
                if min_gallop > 1 {
                    min_gallop -= 1;
                }
                self.min_gallop = min_gallop;
                let mut k = gallop_right(&self.buf, is_lt, &values[cursor_b], cursor_a, len_a, 0)?;
                a_count = k;
                if k > 0 {
                    values[dest..dest + k].clone_from_slice(&self.buf[cursor_a..cursor_a + k]);
                    dest += k;
                    cursor_a += k;
                    len_a -= k;
                    if len_a == 1 {
                        breakout = Some(Breakout::CopyB);
                        break;
                    }
                    if len_a == 0 {
                        breakout = Some(Breakout::Succeed);
                        break;
                    }
                }
                values[dest] = values[cursor_b].clone();
                dest += 1;
                cursor_b += 1;
                len_b -= 1;
                if len_b == 0 {
                    breakout = Some(Breakout::Succeed);
                    break;
                }
                k = gallop_left(&values, is_lt, &self.buf[cursor_a], cursor_b, len_b, 0)?;
                b_count = k;
                if k > 0 {
                    copy_within_clone(values, cursor_b, dest, k);
                    dest += k;
                    cursor_b += k;
                    len_b -= k;
                    if len_b == 0 {
                        breakout = Some(Breakout::Succeed);
                        break;
                    }
                }
                if len_a == 1 {
                    breakout = Some(Breakout::CopyB);
                    break;
                }
                if a_count < MIN_GALLOP && b_count < MIN_GALLOP {
                    break;
                }
            }
            if breakout.is_some() {
                break;
            }
            min_gallop += 1;
            self.min_gallop = min_gallop;
        }

        match breakout {
            Some(Breakout::Succeed) => {
                if len_a > 0 {
                    values[dest..dest + len_a]
                        .clone_from_slice(&self.buf[cursor_a..cursor_a + len_a]);
                }
                Ok(())
            }
            Some(Breakout::CopyB) => {
                copy_within_clone(values, cursor_b, dest, len_b);
                values[dest + len_b] = self.buf[cursor_a].clone();
                Ok(())
            }
            _ => unreachable!(),
        }
    }

    fn merge_hi<E, F>(
        &mut self,
        values: &mut [T],
        is_lt: &mut F,
        start_a: usize,
        mut len_a: usize,
        start_b: usize,
        mut len_b: usize,
    ) -> Result<(), E>
    where
        F: FnMut(&T, &T) -> Result<bool, E>,
    {
        debug_assert!(len_a > 0);
        debug_assert!(len_b > 0);
        debug_assert!(start_a + len_a == start_b);

        self.buf.clear();
        self.buf
            .extend_from_slice(&values[start_b..start_b + len_b]);

        let mut dest = start_b + len_b - 1;
        let mut cursor_a = start_a + len_a - 1;
        let mut cursor_b = len_b - 1;

        values[dest] = values[cursor_a].clone();
        dest -= 1;
        cursor_a -= 1;
        len_a -= 1;

        if len_a == 0 {
            values[dest - len_b + 1..dest + 1].clone_from_slice(&self.buf[0..len_b]);
            return Ok(());
        }
        if len_b == 1 {
            let src = cursor_a + 1 - len_a;
            let dst = dest + 1 - len_a;
            copy_within_clone(values, src, dst, len_a);
            values[dst - 1] = self.buf[cursor_b].clone();
            return Ok(());
        }

        let mut min_gallop = self.min_gallop;
        let mut breakout: Option<Breakout> = None;

        loop {
            let mut a_count = 0;
            let mut b_count = 0;

            loop {
                if is_lt(&self.buf[cursor_b], &values[cursor_a])? {
                    values[dest] = values[cursor_a].clone();
                    dest -= 1;
                    len_a -= 1;

                    if len_a == 0 {
                        breakout = Some(Breakout::Succeed);
                        break;
                    }

                    cursor_a -= 1;
                    a_count += 1;
                    b_count = 0;

                    if a_count >= min_gallop {
                        break;
                    }
                } else {
                    values[dest] = self.buf[cursor_b].clone();
                    dest -= 1;
                    cursor_b -= 1;
                    len_b -= 1;
                    b_count += 1;
                    a_count = 0;
                    if len_b == 1 {
                        breakout = Some(Breakout::CopyA);
                        break;
                    }
                    if b_count >= min_gallop {
                        break;
                    }
                }
            }

            if breakout.is_some() {
                break;
            }

            min_gallop += 1;
            loop {
                if min_gallop > 1 {
                    min_gallop -= 1;
                }
                self.min_gallop = min_gallop;
                let mut k = gallop_right(
                    &values,
                    is_lt,
                    &self.buf[cursor_b],
                    start_a,
                    len_a,
                    len_a - 1,
                )?;
                k = len_a - k;
                a_count = k;
                if k > 0 {
                    copy_within_clone(values, cursor_a + 1 - k, dest + 1 - k, k);
                    dest -= k;
                    len_a -= k;
                    if len_a == 0 {
                        breakout = Some(Breakout::Succeed);
                        break;
                    }
                    cursor_a -= k;
                }
                values[dest] = self.buf[cursor_b].clone();
                dest -= 1;
                cursor_b -= 1;
                len_b -= 1;
                if len_b == 1 {
                    breakout = Some(Breakout::CopyA);
                    break;
                }
                k = gallop_left(&self.buf, is_lt, &values[cursor_a], 0, len_b, len_b - 1)?;
                k = len_b - k;
                b_count = k;
                if k > 0 {
                    values[dest + 1 - k..dest + 1]
                        .clone_from_slice(&self.buf[cursor_b + 1 - k..cursor_b + 1]);
                    dest -= k;
                    len_b -= k;

                    if len_b == 0 {
                        breakout = Some(Breakout::Succeed);
                        break;
                    }
                    cursor_b -= k;

                    if len_b == 1 {
                        breakout = Some(Breakout::CopyA);
                        break;
                    }
                }
                values[dest] = values[cursor_a].clone();
                dest -= 1;
                len_a -= 1;

                if len_a == 0 {
                    breakout = Some(Breakout::Succeed);
                    break;
                }

                cursor_a -= 1;

                if a_count < MIN_GALLOP && b_count < MIN_GALLOP {
                    break;
                }
            }
            if breakout.is_some() {
                break;
            }
            min_gallop += 1;
            self.min_gallop = min_gallop;
        }

        match breakout {
            Some(Breakout::Succeed) => {
                if len_b > 0 {
                    values[dest - len_b + 1..dest + 1].clone_from_slice(&self.buf[0..len_b]);
                }
                Ok(())
            }
            Some(Breakout::CopyA) => {
                let src = cursor_a + 1 - len_a;
                let dst = dest + 1 - len_a;
                copy_within_clone(values, src, dst, len_a);
                values[dst - 1] = self.buf[cursor_b].clone();
                Ok(())
            }
            _ => unreachable!(),
        }
    }

    fn merge_at<E, F>(&mut self, values: &mut [T], is_lt: &mut F, i: usize) -> Result<(), E>
    where
        F: FnMut(&T, &T) -> Result<bool, E>,
    {
        debug_assert!(self.pending.len() >= 2);
        debug_assert!(i == self.pending.len() - 2 || i == self.pending.len() - 3);

        let mut start_a = self.pending[i].base;
        let mut len_a = self.pending[i].len;
        let start_b = self.pending[i + 1].base;
        let mut len_b = self.pending[i + 1].len;

        debug_assert!(len_a > 0);
        debug_assert!(len_b > 0);
        debug_assert!(start_a + len_a == start_b);

        self.pending[i].len = len_a + len_b;
        self.pending.remove(i + 1);

        let k = gallop_right(values, is_lt, &values[start_b], start_a, len_a, 0)?;
        start_a += k;
        len_a -= k;

        if len_a == 0 {
            return Ok(());
        }

        len_b = gallop_left(
            values,
            is_lt,
            &values[start_a + len_a - 1],
            start_b,
            len_b,
            len_b - 1,
        )?;

        if len_b == 0 {
            return Ok(());
        }

        if len_a <= len_b {
            self.merge_lo(values, is_lt, start_a, len_a, start_b, len_b)?;
        } else {
            self.merge_hi(values, is_lt, start_a, len_a, start_b, len_b)?;
        }
        Ok(())
    }

    fn found_new_run<E, F>(&mut self, new_run_len: usize, values: &mut [T], is_lt: &mut F) -> Result<(), E>
    where
        F: FnMut(&T, &T) -> Result<bool, E>,
    {
        if !self.pending.is_empty() {
            let last = self.pending.len() - 1;
            let s1 = self.pending[last].base;
            let n1 = self.pending[last].len;
            let power = powerloop(s1, n1, new_run_len, values.len());

            while self.pending.len() > 1 && self.pending[self.pending.len() - 2].power > power {
                self.merge_at(values, is_lt, self.pending.len() - 2)?;
            }

            debug_assert!(
                self.pending.len() < 2 || self.pending[self.pending.len() - 2].power < power
            );
            let last = self.pending.len() - 1;
            self.pending[last].power = power;
        }
        Ok(())
    }

    fn push_run(&mut self, base: usize, len: usize) {
        self.pending.push(Run {
            base,
            len,
            power: 0,
        })
    }

    fn merge_force_collapse<E, F>(&mut self, values: &mut [T], is_lt: &mut F) -> Result<(), E>
    where
        F: FnMut(&T, &T) -> Result<bool, E>,
    {
        while self.pending.len() > 1 {
            let mut n = self.pending.len() - 2;
            if n > 0 && self.pending[n - 1].len < self.pending[n + 1].len {
                n -= 1;
            }
            self.merge_at(values, is_lt, n)?;
        }
        Ok(())
    }
}

fn binary_insertion_sort<T, E, F>(values: &mut [T], is_lt: &mut F, start: usize) -> Result<(), E>
where
    F: FnMut(&T, &T) -> Result<bool, E>,
{
    for i in start..values.len() {
        let mut l = 0;
        let mut r = i;

        while l < r {
            let m = (l + r) / 2;
            if is_lt(&values[i], &values[m])? {
                r = m;
            } else {
                l = m + 1;
            }
        }
        values[l..=i].rotate_right(1);
    }
    Ok(())
}

fn copy_within_clone<T: Clone>(values: &mut [T], src: usize, dest: usize, n: usize) {
    if dest <= src {
        for k in 0..n {
            values[dest + k] = values[src + k].clone();
        }
    } else {
        for k in (0..n).rev() {
            values[dest + k] = values[src + k].clone();
        }
    }
}

fn count_run<T, E, F>(values: &[T], is_lt: &mut F) -> Result<(usize, bool), E>
where
    F: FnMut(&T, &T) -> Result<bool, E>,
{
    let n = values.len();
    if n == 1 {
        return Ok((1, false));
    }
    let mut i = 2;
    let descending = is_lt(&values[1], &values[0])?;
    if descending {
        while i < n && is_lt(&values[i], &values[i - 1])? {
            i += 1;
        }
    } else {
        while i < n && !is_lt(&values[i], &values[i - 1])? {
            i += 1;
        }
    }
    Ok((i, descending))
}

fn gallop_left<T, E, F>(
    values: &[T],
    is_lt: &mut F,
    key: &T,
    base: usize,
    len: usize,
    hint: usize,
) -> Result<usize, E>
where
    F: FnMut(&T, &T) -> Result<bool, E>,
{
    debug_assert!(hint < len);
    let mut lastofs: isize = 0;
    let mut ofs: isize = 1;
    let hint_i = hint as isize;
    let len_i = len as isize;

    if is_lt(&values[base + hint], key)? {
        let maxofs = len_i - hint_i;
        while ofs < maxofs && is_lt(&values[base + hint + ofs as usize], key)? {
            lastofs = ofs;
            ofs = (ofs * 2) + 1;
        }
        if ofs > maxofs {
            ofs = maxofs;
        }
        lastofs += hint_i;
        ofs += hint_i;
    } else {
        let maxofs = hint_i + 1;
        while ofs < maxofs && !is_lt(&values[base + (hint_i - ofs) as usize], key)? {
            lastofs = ofs;
            ofs = (ofs * 2) + 1;
        }
        if ofs > maxofs {
            ofs = maxofs;
        }
        (lastofs, ofs) = (hint_i - ofs, hint_i - lastofs);
    }
    lastofs += 1;
    while lastofs < ofs {
        let m = lastofs + ((ofs - lastofs) / 2);
        if is_lt(&values[base + m as usize], key)? {
            lastofs = m + 1;
        } else {
            ofs = m;
        }
    }
    Ok(ofs as usize)
}

fn gallop_right<T, E, F>(
    values: &[T],
    is_lt: &mut F,
    key: &T,
    base: usize,
    len: usize,
    hint: usize,
) -> Result<usize, E>
where
    F: FnMut(&T, &T) -> Result<bool, E>,
{
    debug_assert!(hint < len);
    let mut lastofs: isize = 0;
    let mut ofs: isize = 1;
    let hint_i = hint as isize;
    let len_i = len as isize;

    if is_lt(key, &values[base + hint])? {
        let maxofs = hint_i + 1;
        while ofs < maxofs && is_lt(key, &values[base + (hint_i - ofs) as usize])? {
            lastofs = ofs;
            ofs = (ofs * 2) + 1;
        }
        if ofs > maxofs {
            ofs = maxofs;
        }
        (lastofs, ofs) = (hint_i - ofs, hint_i - lastofs);
    } else {
        let maxofs = len_i - hint_i;
        while ofs < maxofs && !is_lt(key, &values[base + hint + ofs as usize])? {
            lastofs = ofs;
            ofs = (ofs * 2) + 1;
        }
        if ofs > maxofs {
            ofs = maxofs;
        }
        lastofs += hint_i;
        ofs += hint_i;
    }
    lastofs += 1;
    while lastofs < ofs {
        let m = lastofs + ((ofs - lastofs) / 2);
        if is_lt(key, &values[base + m as usize])? {
            ofs = m;
        } else {
            lastofs = m + 1;
        }
    }
    Ok(ofs as usize)
}

// TODO: consider CPython 3.12+'s incremental minrun (mr_current/mr_e/mr_mask)
//       for a more precise minrun; current bit-shift version is the classic one.
fn merge_compute_minrun(mut n: usize) -> usize {
    let mut r = 0;
    while n >= MAX_MINRUN {
        r |= n & 1;
        n >>= 1;
    }
    n + r
}

fn powerloop(s1: usize, n1: usize, n2: usize, n: usize) -> u32 {
    let mut result: u32 = 0;
    let mut a = 2 * s1 + n1;
    let mut b = a + n1 + n2;

    loop {
        result += 1;
        if a >= n {
            debug_assert!(b >= a);
            a -= n;
            b -= n;
        } else if b >= n {
            break;
        }
        debug_assert!(a < b && b < n);
        a <<= 1;
        b <<= 1;
    }
    result
}

/// Stable adaptive mergesort (Tim Peters' timsort with powersort's
/// merge-ordering policy, matching CPython 3.11+). `is_lt` provides comparison.
pub(crate) fn timsort<T, E, F>(values: &mut [T], is_lt: &mut F) -> Result<(), E>
where
    T: Clone,
    F: FnMut(&T, &T) -> Result<bool, E>,
{
    let n = values.len();
    let mut ms = MergeState {
        buf: Vec::new(),
        min_gallop: MIN_GALLOP,
        pending: Vec::new(),
    };

    if n < 2 {
        return Ok(());
    }

    if n < MAX_MINRUN {
        let (l, desc) = count_run(&values, is_lt)?;
        if desc {
            values[0..l].reverse();
        }
        binary_insertion_sort(values, is_lt, l)?;
        return Ok(());
    }

    let minrun = merge_compute_minrun(n);
    let mut lo = 0;

    while lo < n {
        let (mut l, desc) = count_run(&values[lo..n], is_lt)?;
        if desc {
            values[lo..lo + l].reverse();
        }
        if l < minrun {
            let force = minrun.min(n - lo);
            binary_insertion_sort(&mut values[lo..lo + force], is_lt, l)?;
            l = force;
        }
        ms.found_new_run(l, values, is_lt)?;
        ms.push_run(lo, l);
        lo += l;
    }
    ms.merge_force_collapse(values, is_lt)?;
    debug_assert!(ms.pending.len() == 1 && ms.pending[0].len == n);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sort(mut v: Vec<i32>) -> Vec<i32> {
        timsort(&mut v, &mut |a: &i32, b: &i32| Ok::<bool, ()>(a < b)).unwrap();
        v
    }

    #[test]
    fn test_basic() {
        assert_eq!(sort(vec![3, 1, 2]), vec![1, 2, 3]);
        assert_eq!(sort(Vec::<i32>::new()), Vec::<i32>::new());
        assert_eq!(sort(vec![1]), vec![1]);
        assert_eq!(sort(vec![2, 1]), vec![1, 2]);
    }

    #[test]
    fn test_ordered() {
        assert_eq!(sort(vec![1, 2, 3, 4, 5]), vec![1, 2, 3, 4, 5]);
        assert_eq!(sort(vec![5, 4, 3, 2, 1]), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_duplicates() {
        assert_eq!(sort(vec![3, 1, 3, 1, 2, 2]), vec![1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn test_large() {
        let mut v: Vec<i32> = (0..1000).rev().collect(); // 999..0
        let sorted: Vec<i32> = (0..1000).collect();
        assert_eq!(sort(v), sorted);
    }

    #[test]
    fn test_random_ish() {
        let mut v: Vec<i32> = (0..500).map(|i| (i * 7919) % 500).collect();
        let mut expected = v.clone();
        expected.sort();
        assert_eq!(sort(v), expected);
    }
}
