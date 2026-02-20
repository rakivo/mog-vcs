use std::collections::{HashMap, HashSet};

use xxhash_rust::xxh3::Xxh3DefaultBuilder;

pub type Xxh3HashSet<K> = HashSet<K, Xxh3DefaultBuilder>;
pub type Xxh3HashMap<K, V> = HashMap<K, V, Xxh3DefaultBuilder>;

#[inline]
#[must_use]
pub fn str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(bytes: &[u8]) -> &str {
    unsafe { core::str::from_utf8_unchecked(bytes) }
}

/// `std::vec::Vec::into_boxed_slice` takes CPU cycles to shrink
/// itself to the `.len`, this function does not shrink and saves
/// us some CPU cycles
#[inline]
#[must_use]
pub fn vec_into_boxed_slice_noshrink<T>(mut v: Vec<T>) -> Box<[T]> {
    let len = v.len();
    let ptr = v.as_mut_ptr();

    core::mem::forget(v);

    unsafe { Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len)) }
}

#[inline]
pub fn stdout_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

#[inline]
pub fn is_executable(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))] {
        false
    }
}

#[macro_export]
macro_rules! payload_triple {
    (
        owned $Owned:ident {
            $( $owned_field:ident : $owned_ty:ty ),+ $(,)?
        }
        view $View:ident <'a> {
            $( $view_field:ident : $view_ty:ty ),+ $(,)?
        }
        ref $Ref:ident <'a> {
            $( $ref_field:ident : $ref_ty:ty ),+ $(,)?
        }
        view_from_owned($owned:ident) $from_owned:block
        view_from_ref($r:ident) $from_ref:block
    ) => {
        pub struct $Owned {
            $( pub $owned_field: $owned_ty, )+
        }

        pub struct $View<'a> {
            $( pub $view_field: $view_ty, )+
        }

        pub struct $Ref<'a> {
            $( pub $ref_field: $ref_ty, )+
        }

        #[allow(unused)]
        impl $Owned {
            #[inline]
            pub fn view(&self) -> $View<'_> {
                let $owned = self;
                $from_owned
            }
        }

        #[allow(unused)]
        impl<'a> $Ref<'a> {
            #[inline]
            pub fn view(self) -> $View<'a> {
                let $r = self;
                $from_ref
            }
        }
    };
}
