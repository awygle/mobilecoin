// Copyright (c) 2018-2022 The MobileCoin Foundation

//! A thin wrapper around Dalek libraries for key handling.
//!
//! # Examples:
//!
//! X25519 ECDH with Ephemeral Keys
//!
//! ```
//! use mc_crypto_keys::*;
//! use mc_util_from_random::FromRandom;
//! use rand_core::SeedableRng;
//! use rand_hc::Hc128Rng;
//!
//! let mut csprng = Hc128Rng::seed_from_u64(0); // but use a real rng
//! let privkey1 = X25519EphemeralPrivate::from_random(&mut csprng);
//! let pubkey1 = X25519Public::from(&privkey1);
//!
//! let privkey2 = X25519EphemeralPrivate::from_random(&mut csprng);
//! let pubkey2 = X25519Public::from(&privkey2);
//!
//! let shared1 = privkey1.key_exchange(&pubkey2);
//! // privkey1 is now dead.
//! let shared2 = privkey2.key_exchange(&pubkey1);
//! // privkey2 is now dead, too.
//!
//! let shared1: &[u8] = shared1.as_ref();
//! let shared2: &[u8] = shared2.as_ref();
//!
//! assert_eq!(shared1, shared2);
//! ```
//!
//! Ed25519 Signing
//!
//! ```
//! use mc_crypto_keys::*;
//! use mc_util_from_random::FromRandom;
//! use rand_core::SeedableRng;
//! use rand_hc::Hc128Rng;
//!
//! let mut csprng = Hc128Rng::seed_from_u64(0); // but use a real rng
//! let pair = Ed25519Pair::from_random(&mut csprng);
//! let signature = pair.sign(b"this is a message, as bytes");
//! let pubkey = pair.public_key();
//!
//! assert!(pubkey.verify(b"this is a message, as bytes", &signature).is_ok());
//! ```

#![no_std]

extern crate alloc;

mod ed25519;
mod ristretto;
mod traits;
mod x25519;

pub use crate::{
    ed25519::{Ed25519Pair, Ed25519Private, Ed25519Public, Ed25519Signature},
    ristretto::{
        CompressedRistrettoPublic, Ristretto, RistrettoEphemeralPrivate, RistrettoPrivate,
        RistrettoPublic, RistrettoSecret, RistrettoSignature,
    },
    traits::{
        DistinguishedEncoding, Fingerprintable, Kex, KexEphemeralPrivate, KexPrivate, KexPublic,
        KexReusablePrivate, KexSecret, KeyError, PrivateKey, PublicKey,
    },
    x25519::{
        X25519EphemeralPrivate, X25519Private, X25519Public, X25519Secret, X25519, X25519_LEN,
    },
};

// Expected format for base64 strings
pub(crate) const B64_CONFIG: base64::Config = base64::STANDARD;

pub use digest::Digest;
pub use mc_util_repr_bytes::{typenum::Unsigned, GenericArray, LengthMismatch, ReprBytes};
pub use signature::{
    DigestSigner, DigestVerifier, Error as SignatureError, Signature, Signer, Verifier,
};
