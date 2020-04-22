// -*- mode: rust; -*-
//
// This file is part of aeonflux.
// Copyright (c) 2020 The Brave Authors
// See LICENSE for licensing information.
//
// Authors:
// - isis agora lovecruft <isis@patternsinthevoid.net>

//! Non-interactive zero-knowledge proofs (NIPKs).

#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::vec::Vec;
#[cfg(all(not(feature = "alloc"), feature = "std"))]
use std::vec::Vec;

use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::traits::Identity;

use rand_core::CryptoRng;
use rand_core::RngCore;

use zkp::CompactProof;
use zkp::Transcript;
// XXX do we want/need batch proof verification?
// use zkp::toolbox::batch_verifier::BatchVerifier;
use zkp::toolbox::prover::PointVar;
use zkp::toolbox::prover::Prover;
use zkp::toolbox::prover::ScalarVar;
// use zkp::toolbox::verifier::Verifier;
use zkp::toolbox::SchnorrCS;

use crate::amacs::Attribute;
use crate::amacs::Messages;
use crate::amacs::SecretKey;
use crate::credential::Credential;
use crate::parameters::{IssuerParameters, SystemParameters};

pub struct ProofOfIssuance(CompactProof);

/// A non-interactive zero-knowledge proof demonstating knowledge of the
/// issuer's secret key, and that a [`Credential`] was computed correctly
/// w.r.t. the pubilshed system and issuer parameters.
impl ProofOfIssuance {
    /// Create a [`ProofOfIssuance`].
    pub fn prove(
        secret_key: &SecretKey,
        system_parameters: &SystemParameters,
        issuer_parameters: &IssuerParameters,
        credential: &Credential,
    ) -> ProofOfIssuance
    {
        let mut transcript = Transcript::new(b"2019/1416 anonymous credential");
        let mut prover = Prover::new(b"2019/1416 issuance proof", &mut transcript);

        // Commit the names of the Camenisch-Stadler secrets to the protocol transcript.
        let w       = prover.allocate_scalar(b"w",   secret_key.w);
        let w_prime = prover.allocate_scalar(b"w'",  secret_key.w_prime);
        let x_0     = prover.allocate_scalar(b"x_0", secret_key.x_0);
        let x_1     = prover.allocate_scalar(b"x_1", secret_key.x_1);

        let mut y: Vec<ScalarVar> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);

        for (i, y_i) in secret_key.y.iter().enumerate() {
            // XXX fix the zkp crate to take Strings
            //y.push(prover.allocate_scalar(format!("y_{}", i), y_i));
            y.push(prover.allocate_scalar(b"y", *y_i));
        }

        // We also have to commit to the multiplicative identity since one of the
        // zero-knowledge statements requires the inverse of the G_V generator
        // without multiplying by any scalar.
        let one = prover.allocate_scalar(b"1", Scalar::one());

        let t = prover.allocate_scalar(b"t", credential.amac.t);

        // Commit to the values and names of the Camenisch-Stadler publics.
        let (neg_G_V, _)   = prover.allocate_point(b"-G_V",     -system_parameters.G_V);
        let (G, _)         = prover.allocate_point(b"G",         system_parameters.G);
        let (G_w, _)       = prover.allocate_point(b"G_w",       system_parameters.G_w);
        let (G_w_prime, _) = prover.allocate_point(b"G_w_prime", system_parameters.G_w_prime);
        let (G_x_0, _)     = prover.allocate_point(b"G_x_0",     system_parameters.G_x_0);
        let (G_x_1, _)     = prover.allocate_point(b"G_x_1",     system_parameters.G_x_1);

        let mut G_y: Vec<PointVar> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);

        for (i, G_y_i) in system_parameters.G_y.iter().enumerate() {
            // XXX fix the zkp crate to take Strings
            //let (G_y_x, _) = prover.allocate_point(format!("G_y_{}", i), G_y_i);
            let (G_y_x, _) = prover.allocate_point(b"G_y", *G_y_i);

            G_y.push(G_y_x);
        }

        let (C_W, _) = prover.allocate_point(b"C_W", issuer_parameters.C_W);
        let (I, _)   = prover.allocate_point(b"I",   issuer_parameters.I);
        let (U, _)   = prover.allocate_point(b"U", credential.amac.U);
        let (V, _)   = prover.allocate_point(b"V", credential.amac.V);

        let mut M: Vec<PointVar> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);

        let messages: Messages = Messages::from_attributes(&credential.attributes, system_parameters);

        for (i, M_i) in messages.0.iter().enumerate() {
            // XXX fix the zkp crate to take Strings
            //let (M_x, _) = prover.allocate_point(format!("M_{}", i), M_i);
            let (M_x, _) = prover.allocate_point(b"M", *M_i);

            M.push(M_x);
        }

        // Constraint #1: C_W = G_w * w + G_w' * w'
        prover.constrain(C_W, vec![(w, G_w), (w_prime, G_w_prime)]);

        // Constraint #2: I = -G_V + G_x_0 * x_0 + G_x_1 * x_1 + G_y_1 * y_1 + ... + G_y_n * y_n
        let mut rhs: Vec<(ScalarVar, PointVar)> = Vec::with_capacity(3 + system_parameters.NUMBER_OF_ATTRIBUTES as usize);

        rhs.push((one, neg_G_V));
        rhs.push((x_0, G_x_0));
        rhs.push((x_1, G_x_1));
        rhs.extend(y.iter().copied().zip(G_y.iter().copied()));

        prover.constrain(I, rhs);

        // Constraint #3: V = G * w + U * x_0 + U * x_1 + U * t + \sigma{i=1}{n} M_i * y_i
        let mut rhs: Vec<(ScalarVar, PointVar)> = Vec::with_capacity(4 + system_parameters.NUMBER_OF_ATTRIBUTES as usize);

        rhs.push((w, G));
        rhs.push((x_0, U));
        rhs.push((x_1, U));
        rhs.push((t, U));
        rhs.extend(y.iter().copied().zip(M.iter().copied()));

        prover.constrain(V, rhs);

        ProofOfIssuance(prover.prove_compact())
    }
}

/// A proof-of-knowledge of a valid `Credential` and its attributes,
/// which may be either hidden or revealed.
pub struct ProofOfValidCredential {
    proof: CompactProof,
    C_x_0: RistrettoPoint,
    C_x_1: RistrettoPoint,
    C_V:   RistrettoPoint,
    C_y: Vec<RistrettoPoint>,
    Z:     RistrettoPoint,
}

impl ProofOfValidCredential {
    /// Create a [`ProofOfValidCredential`]
    pub fn prove<C>(
        system_parameters: &SystemParameters,
        issuer_parameters: &IssuerParameters,
        credential: &Credential,
        csprng: &mut C,
    ) -> ProofOfValidCredential
    where
        C: RngCore + CryptoRng,
    {
        // Choose a nonce for the commitments.
        let z_:   Scalar = Scalar::random(csprng);
        let z_0_: Scalar = (-credential.amac.t * z_).reduce();

        // Commit to the credential attributes.
        let mut C_y_i_: Vec<RistrettoPoint> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);
        let mut M_i_:   Vec<RistrettoPoint> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);
        let mut m_i_: Vec<(usize, RistrettoPoint, Scalar)> = Vec::new();

        for (i, attribute) in credential.attributes.iter().enumerate() {
            let M_i: RistrettoPoint = match attribute {
                Attribute::PublicPoint(_)  => RistrettoPoint::identity(),
                Attribute::SecretPoint(M)  => *M,
                Attribute::PublicScalar(_) => RistrettoPoint::identity(),
                Attribute::SecretScalar(m) => {
                    m_i_.push((i, system_parameters.G_m[i], *m));
                    system_parameters.G_m[i] * m
                },
            };

            M_i_.push(M_i);
            C_y_i_.push((system_parameters.G_y[i] * z_) + M_i);
        }
        let C_x_0_: RistrettoPoint = (system_parameters.G_x_0 * z_) +  credential.amac.U;
        let C_x_1_: RistrettoPoint = (system_parameters.G_x_1 * z_) + (credential.amac.U * credential.amac.t);
        let C_V_:   RistrettoPoint = (system_parameters.G_V   * z_) +  credential.amac.V;
        let Z_:     RistrettoPoint =  issuer_parameters.I     * z_;

        // Create a transcript and prover.
        let mut transcript = Transcript::new(b"2019/1416 anonymous credential");
        let mut prover = Prover::new(b"2019/1416 presentation proof", &mut transcript);

        // Feed the domain separators for the Camenisch-Stadler secrets into the protocol transcript.
        let one = prover.allocate_scalar(b"1", Scalar::one());
        let z   = prover.allocate_scalar(b"z", z_);
        let z_0 = prover.allocate_scalar(b"z_0", z_0_);
        let t = prover.allocate_scalar(b"t", credential.amac.t);

        let mut m_i: Vec<ScalarVar> = Vec::with_capacity(m_i_.len());

        for (i, basepoint, scalar) in m_i_.iter() {
            // XXX Fix zkp crate to take Strings
            //m_i.push(prover.allocate_scalar(format!(b"m_{}", i), scalar));
            m_i.push(prover.allocate_scalar(b"m", *scalar));
        }

        // Feed in the domain separators and values for the publics into the transcript.
        let (Z, _)     = prover.allocate_point(b"Z", Z_);
        let (I, _)     = prover.allocate_point(b"I", issuer_parameters.I);
        let (C_x_1, _) = prover.allocate_point(b"C_x_1", C_x_1_);
        let (C_x_0, _) = prover.allocate_point(b"C_x_0", C_x_0_);
        let (G_x_0, _) = prover.allocate_point(b"G_x_0", system_parameters.G_x_0);
        let (G_x_1, _) = prover.allocate_point(b"G_x_1", system_parameters.G_x_1);

        let mut C_y: Vec<PointVar> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);
        let mut G_y: Vec<PointVar> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);
        let mut M:   Vec<PointVar> = Vec::with_capacity(system_parameters.NUMBER_OF_ATTRIBUTES as usize);

        for (i, commitment) in C_y_i_.iter().enumerate() {
            // XXX Fix zkp crate to take Strings
            //let (C_y_i, _) = prover.allocate_point(format!(b"C_y_{}", i), commitment);
            let (C_y_i, _) = prover.allocate_point(b"C_y", *commitment);

            C_y.push(C_y_i);
        }
        for (i, basepoint) in system_parameters.G_y.iter().enumerate() {
            // XXX Fix zkp crate to take Strings
            // let (G_y_i, _) = prover.allocate_point(format!(b"G_y_{}", i), basepoint);
            let (G_y_i, _) = prover.allocate_point(b"G_y", *basepoint);

            G_y.push(G_y_i);
        }
        for (i, message) in M_i_.iter().enumerate() {
            // XXX Fix zkp crate to take Strings
            //let (M_i, _) = prover.allocate_point(format!(b"M_{}", i), message);
            let (M_i, _) = prover.allocate_point(b"M", *message);

            M.push(M_i);
        }

        // Constraint #1: Z = I * z
        prover.constrain(Z, vec![(z, I)]);

        // Constraint #2: C_x_1 = C_x_0 * t          + G_x_0 * z_0 + G_x_1 * z
        //    G_x_1 * z + U * t = G_x_0 * zt + U * t + G_x_0 * -tz + G_x_1 * z
        //    G_x_1 * z + U * t =              U * t +               G_x_1 * z
        prover.constrain(C_x_1, vec![(t, C_x_0), (z_0, G_x_0), (z, G_x_1)]);

        //                        { G_y_i * z + M_i                  if i is a hidden group attribute
        // Constraint #3: C_y_i = { G_y_i * z + G_m_i * m_i          if i is a hidden scalar attribute
        //                        { G_y_i * z                        if i is a revealed attribute
        for (i, C_y_i) in C_y.iter().enumerate() {
            prover.constrain(*C_y_i, vec![(z, G_y[i]), (one, M[i])]);
        }

        let proof = prover.prove_compact();

        ProofOfValidCredential {
            proof: proof,
            C_x_0: C_x_0_,
            C_x_1: C_x_1_,
            C_V: C_V_,
            C_y: C_y_i_,
            Z: Z_,
        }
    }
}
