#![allow(dead_code)]

//! Types holding information about generates Rust API.

use crate::cpp_data::CppPath;
use crate::cpp_data::CppTypeDoc;
use crate::cpp_ffi_data::CppFfiFunction;
use crate::cpp_function::CppFunctionDoc;
use crate::rust_type::{CompleteType, RustPath, RustType};
use serde_derive::{Deserialize, Serialize};

/// One variant of a Rust enum
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct RustEnumValue {
    /// Identifier
    pub name: String,
    /// Corresponding value
    pub value: i64,
    /// Documentation of corresponding C++ variants
    pub cpp_doc: CppEnumValueDoc,
}

/// C++ documentation data for a enum variant
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct CppEnumValueDoc {
    /// C++ name of the variant
    pub variant_name: String,
    /// HTML content
    pub doc: Option<String>,
}

/// Information about a Qt slot wrapper on Rust side
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct RustQtSlotWrapper {
    /// Argument types of the slot
    pub arguments: Vec<CompleteType>,
    /// Identifier of the slot for `QObject::connect`
    pub receiver_id: String,
    /// Name of the extern callback function of this wrapper
    pub callback_path: RustPath,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum RustWrapperTypeKind {
    EnumWrapper,
    ImmovableClassWrapper { raw_type_path: RustPath },
    MovableClassWrapper { verified_type_path: RustPath },
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct RustWrapperType {
    /// Full name and template arguments of corresponding C++ type (class or enum).
    pub cpp_path: CppPath,
    /// C++ documentation for this type
    pub cpp_doc: Option<CppTypeDoc>,
    pub kind: RustWrapperTypeKind,
}

/// Information about a Rust type wrapper
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum RustStructKind {
    WrapperType(RustWrapperType),
    QtSlotWrapper(RustQtSlotWrapper),
    SignalsOrSlots { target_path: RustPath },
}

/// Exported information about a Rust wrapper type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustStruct {
    /// Additional documentation content that will appear before C++ documentation or any other
    /// automatically generated content.
    pub rust_doc: Option<String>,
    pub path: RustPath,
    /// Kind of the type and additional information.
    pub kind: RustStructKind,
    /// Indicates whether this type is public
    pub is_public: bool,
}

/// Information for generating Rust documentation for a method
/// or an item of information for an overloaded method.
/// One value of `RustMethodDocItem` corresponds to a single
/// C++ method.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustFunctionDoc {
    /// Rustdoc content that will appear before documentation for variants.
    pub common_doc: Option<String>,
    /// C++ documentation of the corresponding C++ method.
    pub doc: Option<CppFunctionDoc>,
    /// C++ code containing declaration of the corresponding C++ method.
    pub cpp_fn: String,
}

/// Location of a Rust method.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RustFunctionScope {
    /// Inside `impl T {}`, where `T` is `target_type`.
    Impl { target_type: RustType },
    /// Inside a trait implementation.
    TraitImpl,
    /// A free function.
    Free,
}

/// Information about a Rust method argument.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustFunctionArgument {
    /// C++ and Rust types corresponding to this argument at all levels.
    pub argument_type: CompleteType,
    /// Rust argument name.
    pub name: String,
    /// Index of the corresponding argument of the FFI function.
    pub ffi_index: usize,
}

/// Type of a receiver in Qt connection system.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RustQtReceiverType {
    Signal,
    Slot,
}

#[allow(clippy::large_enum_variant)]
pub enum RustFunctionKind {
    FfiWrapper {
        /// C++ method corresponding to this variant.
        cpp_ffi_method: CppFfiFunction,
        /// Index of the FFI function argument used for acquiring the return value,
        /// if any. `None` if the return value is passed normally (as the return value
        /// of the FFI function).
        return_type_ffi_index: Option<usize>,
    },
    CppDeletableImpl {
        deleter: RustPath,
    },
    SignalOrSlotGetter {
        /// Name of the type.
        type_path: RustPath,
        /// C++ name of the signal or slot
        original_method_name: String,
        /// Type of the receiver.
        receiver_type: RustQtReceiverType,
        /// Identifier of the signal or slot for passing to `QObject::connect`.
        receiver_id: String,
        /// Types or arguments.
        arguments: Vec<RustType>,
    },
}

/// Information about a public API method.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustFunction {
    /// Location of the method.
    pub scope: RustFunctionScope,
    /// True if the method is `unsafe`.
    pub is_unsafe: bool,
    /// Full name of the method.
    pub path: RustPath,

    /// List of arguments. For an overloaded method, only the arguments
    /// involved in the overloading are listed in this field.
    /// There can also be arguments shared by all variants (typically the
    /// `self` argument), and they are not listed in this field.
    pub arguments: Vec<RustFunctionArgument>,
    /// C++ and Rust return types at all levels.
    pub return_type: CompleteType,

    /// Documentation data.
    pub doc: RustFunctionDoc,
}

/// Information about type of `self` argument of the method.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum RustFunctionSelfArgKind {
    /// No `self` argument (static method or a free function).
    None,
    /// `&self` argument.
    ConstRef,
    /// `&mut self` argument.
    MutRef,
    /// `self` argument.
    Value,
}

/// Information about an associated type value
/// within a trait implementation.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TraitAssociatedType {
    /// Name of the associated type.
    pub name: String,
    /// Value of the associated type.
    pub value: RustType,
}

/// Information about a trait implementation.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TraitImpl {
    /// Type the trait is implemented for.
    pub target_type: RustType,
    /// Type of the trait.
    pub trait_type: RustType,
    /// Values of associated types of the trait.
    pub associated_types: Vec<TraitAssociatedType>,
    /// Functions that implement the trait.
    pub functions: Vec<RustFunction>,
}

/// Information about a Rust module.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustModule {
    /// Last name of the module.
    pub path: RustPath,
    /// Markdown content of Rust documentation for this module.
    pub doc: Option<String>,
}

/// Method of generating name suffixes for disambiguating multiple Rust methods
/// with the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustFunctionCaptionStrategy {
    /// Only type of `self` is used.
    SelfOnly,
    /// Unsafe methods have `unsafe` suffix, and safe methods have no suffix.
    UnsafeOnly,
    /// Type of `self` and types of other arguments are used.
    SelfAndArgTypes,
    /// Type of `self` and names of other arguments are used.
    SelfAndArgNames,
    /// Type of `self` and index of method are used.
    SelfAndIndex,
}

impl RustFunctionCaptionStrategy {
    /// Returns list of all available strategies sorted by priority
    /// (more preferred strategies go first).
    pub fn all() -> &'static [RustFunctionCaptionStrategy] {
        use self::RustFunctionCaptionStrategy::*;
        &[
            SelfOnly,
            UnsafeOnly,
            SelfAndArgTypes,
            SelfAndArgNames,
            SelfAndIndex,
        ]
    }
}

/// Information about an argument of a Rust FFI function.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RustFFIArgument {
    /// Name of the argument.
    pub name: String,
    /// Type of the argument.
    pub argument_type: RustType,
}

/// Information about a Rust FFI function.
/// Name and signature of this function must be the same
/// as the corresponding C++ function on the other side of FFI.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RustFFIFunction {
    /// Return type of the function.
    pub return_type: RustType,
    /// Name of the function.
    pub path: RustPath,
    /// Arguments of the function.
    pub arguments: Vec<RustFFIArgument>,
}