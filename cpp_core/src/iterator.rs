use crate::ops::{Decrement, Increment, Indirection};
use crate::{CppBox, CppDeletable, Ref};
use std::os::raw::c_char;

/// `Iterator` and `DoubleEndedIterator` backed by C++ iterators.
///
/// This object is produced by `IntoIterator` implementations on  pointer types
/// (`&CppBox`, `&mut CppBox`, `Ptr`, `Ref`). You can also use
/// `cpp_iter` function to construct it manually from two C++ iterator objects.
pub struct CppIterator<T1, T2>
where
    T1: CppDeletable,
    T2: CppDeletable,
{
    begin: CppBox<T1>,
    end: CppBox<T2>,
}

/// Constructs a Rust-style iterator from C++ iterators pointing to begin and end
/// of the collection.
///
/// ### Safety
///
/// `begin` and `end` must be valid. It's not possible to make any guarantees about safety, since
/// `CppIterator` will call arbitrary C++ library code when used.
pub unsafe fn cpp_iter<T1, T2>(begin: CppBox<T1>, end: CppBox<T2>) -> CppIterator<T1, T2>
where
    T1: CppDeletable,
    T2: CppDeletable,
{
    CppIterator { begin, end }
}

impl<T1, T2> Iterator for CppIterator<T1, T2>
where
    T1: CppDeletable + PartialEq<Ref<T2>> + Indirection + Increment,
    T2: CppDeletable,
{
    type Item = <T1 as Indirection>::Output;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            if self.begin == self.end.as_ref() {
                None
            } else {
                let inner = &mut *self.begin.as_mut_raw_ptr();
                let value = inner.indirection();
                let inner = &mut *self.begin.as_mut_raw_ptr();
                inner.inc();
                Some(value)
            }
        }
    }
}

impl<T1, T2> DoubleEndedIterator for CppIterator<T1, T2>
where
    T1: CppDeletable + PartialEq<Ref<T2>> + Indirection + Increment,
    T2: CppDeletable + Decrement + Indirection<Output = <T1 as Indirection>::Output>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        unsafe {
            if self.begin == self.end.as_ref() {
                None
            } else {
                let inner = &mut *self.end.as_mut_raw_ptr();
                inner.dec();
                let inner = &mut *self.end.as_mut_raw_ptr();
                let value = inner.indirection();
                Some(value)
            }
        }
    }
}

/// A convenience trait that provides `end_ptr()` method for slices.
pub trait EndPtr {
    /// Type of item.
    type Item;
    /// Returns pointer to the end of the slice (past the last element).
    fn end_ptr(&self) -> *const Self::Item;
}

impl<'a, T> EndPtr for &'a [T] {
    type Item = T;
    fn end_ptr(&self) -> *const T {
        unsafe { self.as_ptr().add(self.len()) }
    }
}

impl<'a> EndPtr for &'a str {
    type Item = c_char;
    fn end_ptr(&self) -> *const c_char {
        unsafe { self.as_ptr().add(self.len()) as *const c_char }
    }
}
