use std::fmt::{self, Debug};
use std::ops::{Add, AddAssign, Range, Sub, SubAssign};
use std::str;

use super::metrics::ByteMetric;
use super::utils::*;
use crate::tree::{Leaf, ReplaceableLeaf, Summarize};

#[cfg(all(not(test), not(feature = "integration_tests")))]
const ROPE_CHUNK_MAX_BYTES: usize = 1024;

#[cfg(any(test, feature = "integration_tests"))]
const ROPE_CHUNK_MAX_BYTES: usize = 4;

const ROPE_CHUNK_MIN_BYTES: usize = ROPE_CHUNK_MAX_BYTES / 2;

#[derive(Clone)]
pub(super) struct RopeChunk {
    pub(super) text: String,
}

impl Debug for RopeChunk {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.text)
    }
}

impl Default for RopeChunk {
    #[inline]
    fn default() -> Self {
        Self { text: String::with_capacity(Self::chunk_max()) }
    }
}

impl RopeChunk {
    /// The number of bytes `RopeChunk`s must always stay under.
    pub(super) const fn chunk_max() -> usize {
        // We can exceed the max by 3 bytes at most, which happens when the
        // chunk boundary would have landed after the first byte of a 4 byte
        // codepoint.
        Self::max_bytes() + 3
    }

    /// The number of bytes `RopeChunk`s must always stay over.
    pub(super) const fn chunk_min() -> usize {
        if Self::min_bytes() > 3 {
            // The wiggle room is 3 bytes for the reason already explained in
            // the comment above.
            Self::min_bytes() - 3
        } else {
            1
        }
    }

    /// The number of bytes `RopeChunk`s should aim to stay under.
    pub(super) const fn max_bytes() -> usize {
        ROPE_CHUNK_MAX_BYTES
    }

    /// The number of bytes `RopeChunk`s should aim to stay over.
    pub(super) const fn min_bytes() -> usize {
        ROPE_CHUNK_MIN_BYTES
    }

    /// Splits the chunk at the given byte offset, returning the right side of
    /// the split. The chunk will have `byte_offset` bytes after splitting.
    ///
    /// # Safety
    ///
    /// The function is unsafe because it does not check that `byte_offset` is
    /// a valid char boundary, leaving that up to the caller.
    unsafe fn split_off_unchecked(&mut self, byte_offset: usize) -> Self {
        debug_assert!(self.is_char_boundary(byte_offset));
        let rhs = self.as_mut_vec().split_off(byte_offset);
        Self { text: String::from_utf8_unchecked(rhs) }
    }

    /// Removes all the contents after `byte_offset` from the chunk, leaving
    /// the capacity untouched.
    ///
    /// # Safety
    ///
    /// The function is unsafe because it does not check that `byte_offset` is
    /// a valid char boundary, leaving that up to the caller.
    unsafe fn truncate_unchecked(&mut self, byte_offset: usize) {
        debug_assert!(self.is_char_boundary(byte_offset));
        self.as_mut_vec().truncate(byte_offset);
    }
}

impl std::borrow::Borrow<ChunkSlice> for RopeChunk {
    #[inline]
    fn borrow(&self) -> &ChunkSlice {
        (&*self.text).into()
    }
}

impl std::ops::Deref for RopeChunk {
    type Target = String;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl std::ops::DerefMut for RopeChunk {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.text
    }
}

impl Summarize for RopeChunk {
    type Summary = ChunkSummary;

    #[inline]
    fn summarize(&self) -> Self::Summary {
        ChunkSummary {
            bytes: self.text.len(),
            line_breaks: str_indices::lines_lf::count_breaks(&self.text),
        }
    }
}

impl Leaf for RopeChunk {
    type BaseMetric = ByteMetric;

    type Slice = ChunkSlice;

    #[inline]
    fn is_big_enough(&self, summary: &ChunkSummary) -> bool {
        summary.bytes >= RopeChunk::min_bytes()
    }

    #[inline]
    fn balance_slices<'a>(
        (left, left_summary): (&'a ChunkSlice, &'a ChunkSummary),
        (right, right_summary): (&'a ChunkSlice, &'a ChunkSummary),
    ) -> ((Self, ChunkSummary), Option<(Self, ChunkSummary)>) {
        if left.len() >= Self::min_bytes() && right.len() >= Self::min_bytes()
        {
            (
                (left.to_owned(), *left_summary),
                Some((right.to_owned(), *right_summary)),
            )
        }
        // If both slices can fit in a single chunk we join them.
        else if left.len() + right.len() <= Self::max_bytes() {
            let mut left = left.to_owned();
            left.push_str(right);

            let mut left_summary = *left_summary;
            left_summary += right_summary;

            ((left, left_summary), None)
        }
        // If the left side is lacking we take text from the right side.
        else if left.len() < Self::min_bytes() {
            debug_assert!(right.len() > Self::min_bytes());

            let (left, right) = balance_left_with_right(
                left,
                left_summary,
                right,
                right_summary,
            );

            (left, Some(right))
        }
        // Viceversa, if the right side is lacking we take text from the left
        // side.
        else {
            debug_assert!(right.len() < Self::min_bytes());
            debug_assert!(left.len() > Self::min_bytes());

            let (left, right) = balance_right_with_left(
                left,
                left_summary,
                right,
                right_summary,
            );

            (left, Some(right))
        }
    }
}

#[derive(Debug, PartialEq)]
#[repr(transparent)]
pub(super) struct ChunkSlice {
    text: str,
}

impl Default for &ChunkSlice {
    #[inline]
    fn default() -> Self {
        "".into()
    }
}

impl From<&str> for &ChunkSlice {
    #[inline]
    fn from(text: &str) -> Self {
        // SAFETY: a `ChunkSlice` has the same layout as a `str`.
        unsafe { &*(text as *const str as *const ChunkSlice) }
    }
}

impl std::ops::Deref for ChunkSlice {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl Summarize for ChunkSlice {
    type Summary = ChunkSummary;

    #[inline]
    fn summarize(&self) -> Self::Summary {
        ChunkSummary {
            bytes: self.text.len(),
            line_breaks: str_indices::lines_lf::count_breaks(&self.text),
        }
    }
}

impl ToOwned for ChunkSlice {
    type Owned = RopeChunk;

    #[inline]
    fn to_owned(&self) -> Self::Owned {
        RopeChunk { text: self.text.to_owned() }
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub(super) struct ChunkSummary {
    pub(super) bytes: usize,
    pub(super) line_breaks: usize,
}

impl Add<Self> for ChunkSummary {
    type Output = Self;

    #[inline]
    fn add(mut self, rhs: Self) -> Self {
        self += &rhs;
        self
    }
}

impl Sub<Self> for ChunkSummary {
    type Output = Self;

    #[inline]
    fn sub(mut self, rhs: Self) -> Self {
        self -= &rhs;
        self
    }
}

impl Add<&Self> for ChunkSummary {
    type Output = Self;

    #[inline]
    fn add(mut self, rhs: &Self) -> Self {
        self += rhs;
        self
    }
}

impl Sub<&Self> for ChunkSummary {
    type Output = Self;

    #[inline]
    fn sub(mut self, rhs: &Self) -> Self {
        self -= rhs;
        self
    }
}

impl AddAssign<&Self> for ChunkSummary {
    #[inline]
    fn add_assign(&mut self, rhs: &Self) {
        self.bytes += rhs.bytes;
        self.line_breaks += rhs.line_breaks;
    }
}

impl SubAssign<&Self> for ChunkSummary {
    #[inline]
    fn sub_assign(&mut self, rhs: &Self) {
        self.bytes -= rhs.bytes;
        self.line_breaks -= rhs.line_breaks;
    }
}

impl ReplaceableLeaf<ByteMetric> for RopeChunk {
    type ExtraLeaves = std::vec::IntoIter<Self>;

    #[inline]
    fn replace(
        &mut self,
        summary: &mut ChunkSummary,
        range: Range<ByteMetric>,
        mut slice: &ChunkSlice,
    ) -> Option<Self::ExtraLeaves> {
        let start = range.start.0;

        let end = range.end.0;

        if self.len() - (end - start) + slice.len() <= Self::max_bytes() {
            if end > start {
                // Compute the summary of the replaced range, either directly
                // or by subtraction depending on which is cheaper.
                let range_summary = if end - start < self.len() / 2 {
                    <&ChunkSlice>::from(&self[start..end]).summarize()
                } else {
                    let up_to_start =
                        <&ChunkSlice>::from(&self[..start]).summarize();

                    let from_end =
                        <&ChunkSlice>::from(&self[end..]).summarize();

                    *summary - (up_to_start + from_end)
                };

                *summary -= &range_summary;
                *summary += &slice.summarize();
                self.replace_range(start..end, slice);
            } else {
                *summary += &slice.summarize();
                self.insert_str(start, slice);
            }

            return None;
        }

        // SAFETY: `end` is a char boundary.
        let mut last = unsafe { self.split_off_unchecked(end) };

        // SAFETY: `start` is a char boundary.
        unsafe { self.truncate_unchecked(start) };

        debug_assert!(
            self.len() + slice.len() + last.len() > Self::max_bytes()
        );

        let mut first = None;

        if self.len() < Self::min_bytes() {
            let mut missing = Self::min_bytes() - self.len();

            // The number of bytes to take from the start of `slice` and add to
            // `self`.
            let take_from_slice = if missing > slice.len() {
                slice.len()
            } else {
                adjust_split_point::<true>(slice, missing)
            };

            self.push_str(&slice[..take_from_slice]);
            slice = slice[take_from_slice..].into();

            // If the slice alone wasn't enough we need to take from `last`.
            if missing > take_from_slice {
                missing -= take_from_slice;

                let take_from_last =
                    adjust_split_point::<true>(&last, missing);

                self.push_str(&last[..take_from_last]);
                last.replace_range(..take_from_last, "");
            }
        } else if slice.len() + last.len() < Self::min_bytes() {
            let missing = Self::min_bytes() - (slice.len() + last.len());

            // We don't have to check that `self.len() > missing` because if we
            // get here we have:
            //
            // ```
            // a) self + slice + last > max_bytes = 2 * min_bytes
            // b) slice + last < min_bytes
            // ```
            //
            // =>
            //
            // ```
            // a) self > 2 * min_bytes - slice - last
            // b) -slice -last > -min_bytes
            // ```
            //
            // and by substituting b) into a) we get
            //
            // ```
            // self > 2 * min_bytes - min_bytes = min_bytes > missing
            // ```

            let keep_in_self =
                adjust_split_point::<true>(&self, self.len() - missing);

            // SAFETY: `keep_in_self` is a valid char boundary.
            first = Some(unsafe { self.split_off_unchecked(keep_in_self) });
        }

        debug_assert!(
            self.len() >= Self::min_bytes() && self.len() <= Self::max_bytes()
        );

        *summary = self.summarize();

        debug_assert!(
            first.as_ref().map(|f| f.len()).unwrap_or(0)
                + slice.len()
                + last.len()
                >= Self::chunk_min()
        );

        let extras = extra_leaves::ExtraLeaves::new(first, slice, last);

        // TODO: implement `ExactSizeIterator` on `ExtraLeaves` and see if it
        // makes any difference.
        //
        // We collect into a Vec because `ExtraLeaves` is not an
        // `ExactSizeIterator`.
        Some(extras.collect::<Vec<_>>().into_iter())
    }
}

pub(super) struct RopeChunkIter<'a> {
    text: &'a str,
    yielded: usize,
}

impl<'a> RopeChunkIter<'a> {
    #[inline]
    pub(super) fn new(text: &'a str) -> Self {
        Self { text, yielded: 0 }
    }
}

impl<'a> Iterator for RopeChunkIter<'a> {
    type Item = &'a str;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let mut remaining = self.text.len() - self.yielded;

        let chunk = if remaining == 0 {
            return None;
        } else if remaining > RopeChunk::max_bytes() {
            let mut chunk_len = RopeChunk::max_bytes();

            remaining -= chunk_len;

            if remaining < RopeChunk::min_bytes() {
                chunk_len -= RopeChunk::min_bytes() - remaining;
            }

            chunk_len = adjust_split_point::<true>(
                &self.text[self.yielded..],
                chunk_len,
            );

            &self.text[self.yielded..(self.yielded + chunk_len)]
        } else {
            debug_assert!(
                self.yielded == 0 || remaining >= RopeChunk::chunk_min()
            );

            &self.text[self.text.len() - remaining..]
        };

        self.yielded += chunk.len();

        Some(chunk)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let lo = (self.text.len() - self.yielded) / RopeChunk::max_bytes();
        (lo, Some(lo + 1))
    }
}

mod extra_leaves {
    use super::*;

    /// TODO: docs
    pub(super) struct ExtraLeaves<'a> {
        first: Option<RopeChunk>,
        yielded_first: bool,
        slice: &'a ChunkSlice,
        last: RopeChunk,
        yielded: usize,
        total: usize,
    }

    impl<'a> ExtraLeaves<'a> {
        #[inline]
        pub(super) fn new(
            first: Option<RopeChunk>,
            slice: &'a ChunkSlice,
            last: RopeChunk,
        ) -> Self {
            Self {
                total: slice.len() + last.len(),
                yielded: 0,
                yielded_first: false,
                first,
                slice,
                last,
            }
        }

        #[inline]
        fn first(&mut self) -> RopeChunk {
            debug_assert!(!self.yielded_first);

            self.yielded_first = true;

            if let Some(mut first) = self.first.take() {
                debug_assert!(first.len() < RopeChunk::min_bytes());

                if self.total <= RopeChunk::max_bytes() {
                    first.push_str(self.slice);
                    first.push_str(&self.last);
                    first
                } else {
                    // let mut missing = RopeChunk::max_bytes() -
                    // let take_from_slice =
                    todo!();
                }
            } else {
                self.next().unwrap()
            }
        }

        #[inline]
        fn next(&mut self) -> Option<RopeChunk> {
            debug_assert!(self.yielded_first);
            debug_assert!(self.first.is_none());

            let mut remaining = self.total - self.yielded;

            let chunk: RopeChunk = if remaining == 0 {
                return None;
            } else if remaining > RopeChunk::max_bytes() {
                todo!();
            } else {
                debug_assert!(remaining >= RopeChunk::chunk_min());

                // if remaining > self.last.len() {
                //     // self[slice]
                // } else {
                // }

                todo!();
            };

            self.yielded += chunk.len();

            Some(chunk)
        }
    }

    impl Iterator for ExtraLeaves<'_> {
        type Item = RopeChunk;

        #[inline]
        fn next(&mut self) -> Option<Self::Item> {
            if !self.yielded_first {
                Some(self.first())
            } else {
                self.next()
            }
        }

        #[inline]
        fn size_hint(&self) -> (usize, Option<usize>) {
            let lo = (self.total - self.yielded) / RopeChunk::max_bytes();
            (lo, Some(lo + 1))
        }
    }
}
