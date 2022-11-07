use sbor::v1::DefaultInterpretations;
use super::EncodeError;
use super::decoder::{Decoder, DecodeError};
use super::encoder::Encoder;

/// Provides the interpretation of the payload.
///
/// Most types/impls will have a fixed interpretation, and can just set the associated const INTERPRETATION.
///
/// Some types/impls will have a dynamic interpration, or can support decoding from multiple interpretations,
/// and can override the get_interpretation / check_interpretation methods.
pub trait Interpretation {
    /// The const INTERPRETATION of the type/impl, or can be set to 0 = DefaultInterpretations::NOT_FIXED
    /// which denotes that the interepretation of the type can be multiple values.
    const INTERPRETATION: u8;

    /// This should be false for all types T except those where their Vec<T> should be turned
    /// into RawBytes, via unsafe direct pointer access. This is only valid for u8/i8 types.
    const IS_BYTE: bool = false;

    #[inline]
    fn get_interpretation(&self) -> u8 {
        if Self::INTERPRETATION == DefaultInterpretations::NOT_FIXED {
            todo!("The get_interpretation method must be overridden if the interpretation is not fixed!")
        }
        Self::INTERPRETATION
    }

    #[inline]
    fn check_interpretation(actual: u8) -> Result<(), DecodeError> {
        if Self::INTERPRETATION == DefaultInterpretations::NOT_FIXED {
            todo!("The check_interpretation method must be overridden if the interpretation is not fixed!")
        }
        check_matching_interpretation(Self::INTERPRETATION, actual)
    }
}

pub trait Schema {
    /// This should denote a unique identifier for this type, in particular capturing the uniqueness of
    /// anything which may be attached to the schema, for example:
    /// * Decode schema - ie what it can decode successfully
    /// * Type recreation
    /// * Any display attachment
    const SCHEMA_TYPE_ID: SchemaTypeId = generate_type_id(stringify!(Self), &[], &[]);
}

type SchemaTypeId = [u8; 20];

pub const fn generate_type_id(name: &str, code: &[u8], dependencies: &[SchemaTypeId]) -> SchemaTypeId {
    let buffer = const_sha1::ConstBuffer::from_slice(name.as_bytes())
        .push_slice(&code);
    // Const Looping isn't allowed - but we can use recursion instead: https://rust-lang.github.io/rfcs/2344-const-looping.html
    let buffer = add_each_dependency(buffer, 0, dependencies);
    const_sha1::sha1(&buffer).bytes()
}

const fn add_each_dependency(buffer: const_sha1::ConstBuffer, next: usize, dependencies: &[SchemaTypeId]) -> const_sha1::ConstBuffer {
    if next == dependencies.len() {
        return buffer;
    }
    add_each_dependency(
        buffer.push_slice(dependencies[next].as_slice()),
        next + 1,
        dependencies
    )
}

pub fn check_matching_interpretation(expected: u8, actual: u8) -> Result<(), DecodeError> {
    if expected == actual {
        Ok(())
    } else {
        Err(DecodeError::InvalidInterpretation { expected, actual })
    }
}

/// The trait representing that the value can be encoded with SBOR.
/// 
/// If implementing Encode, you should also implement Interpretation.
///
/// If using Encode as a type constraint, you have two options:
/// * If the type constraint is to implement Encode, use Encode + Interpretation (to match your Intepretation bound)
/// * If the type constraint is for a method, choose Encode + ?Sized - this enables you to take trait objects, slices etc
pub trait Encode<E: Encoder>: XXInternalHasInterpretation {
    /// Encodes the value (should not encode the interpretation)
    fn encode_value(&self, encoder: &mut E) -> Result<(), EncodeError>;
}

/// The trait representing a decode-target for an SBOR payload
pub trait Decode<D: Decoder>: Interpretation + Sized {
    /// Decodes the value (the interpretation has already been decoded/checked)
    ///
    /// Typically `decode_value` is implemented, unless the interpretation is required
    #[inline]
    fn decode_value_with_interpretation(decoder: &mut D, _read_interpretation: u8) -> Result<Self, DecodeError> {
        Self::decode_value(decoder)
    }

    /// Decodes the value (the interpretation has already been decoded/checked)
    /// 
    /// Typically this is the method which is implemented.
    /// If a type implements decode_value_with_interpretation, decode_value can be implemented with a panic.
    fn decode_value(decoder: &mut D) -> Result<Self, DecodeError>;
}

/// This trait is not intended to be implemented directly - instead, implement the
/// Encode and Decode traits.
pub trait Codec<E: Encoder, D: Decoder>: Encode<E> + Decode<D> {}
impl<T: Encode<E> + Decode<D>, E: Encoder, D: Decoder> Codec<E, D> for T {}

/// Important: This trait is never intended to be implemented directly - instead, implement
/// the `Interpretation` trait.
/// 
/// The HasInterpretation trait creates some slight-redirection, so that Encode does not
/// rely explicitly on the Interpretation trait. This ensures that Encode has no direct
/// associated types (such as the various constants and methods on Intepretation which
/// don't take &self), and so allows for it to be boxed in a trait object.
/// 
/// This means traits doing a blanket impl on T: Encode should actually use T: Encode + Implementation
/// bound to match their T: Implementation bound of their impl of their Implementation trait.
/// 
/// NOTE: It might be compelling to create a ChecksInterpretation trait, and make
/// Decode: ChecksInterpretation -- and having blanket impls only implement these traits and
/// not the Interpretation trait. This doesn't work though - because the blanket impls potentially
/// clash with downstream crates impls for fundamental types such as Box<T>.
pub trait XXInternalHasInterpretation {
    fn get_interpretation(&self) -> u8;
}

impl<T: Interpretation + ?Sized> XXInternalHasInterpretation for T {
    #[inline]
    fn get_interpretation(&self) -> u8 {
        T::get_interpretation(&self)
    }
}
