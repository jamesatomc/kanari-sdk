# 9. References

The design uses established standards and research as design inputs. These references explain the underlying primitives; they do not certify the Kanari implementation.

1. Babel, Chursin, Danezis et al., **Mysticeti: Reaching the Limits of Latency with Uncertified DAGs**, arXiv:2310.14821 (2023). <https://arxiv.org/abs/2310.14821>
2. NIST, **FIPS 204: Module-Lattice-Based Digital Signature Standard (ML-DSA)** (13 Aug 2024). <https://csrc.nist.gov/pubs/fips/204/final>
3. NIST, **FIPS 205: Stateless Hash-Based Digital Signature Standard (SLH-DSA)** (13 Aug 2024). <https://csrc.nist.gov/pubs/fips/205/final>
4. Josefsson and Liusvaara, **RFC 8032: Edwards-Curve Digital Signature Algorithm (EdDSA)** (2017). <https://www.rfc-editor.org/rfc/rfc8032>
5. Biryukov, Dinu, Khovratovich, and Josefsson, **RFC 9106: Argon2 Memory-Hard Function for Password Hashing and Proof-of-Work Applications** (2021). <https://www.rfc-editor.org/rfc/rfc9106>
6. Facebook/Meta, **RocksDB Overview and Recovery Notes**. <https://github.com/facebook/rocksdb/wiki/RocksDB-Overview>
7. The Move Book, **Ownership, Object Model, and Fast Path**. <https://move-book.com/>
8. NIST, **Post-Quantum Cryptography FAQ and validation material**. <https://csrc.nist.gov/Projects/post-quantum-cryptography>

## Citation policy

Normative protocol behavior is defined by the Kanari source code, tests, and versioned migration notes. External papers and standards are cited to explain terminology and security assumptions only. When an implementation differs from a cited paper or standard, this document intentionally describes the implementation rather than implying equivalence.
