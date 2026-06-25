from .common import read, write


def apply():
    path = "crates/mysticeti/crates/dag/src/crypto.rs"
    text = read(path)
    old = '''        if !self.enabled {
            return (
                SignatureBytes::dummy(),
                BlockDigest::synthetic(round, authority),
            );
        }
        let content_hash = BlockDigest::new(authority, round, includes, transactions, timestamp_ns);'''
    new = '''        let content_hash = BlockDigest::new(authority, round, includes, transactions, timestamp_ns);
        if !self.enabled {
            let signature = SignatureBytes::dummy();
            return (signature, content_hash.with_signature(&signature));
        }'''
    if old not in text:
        raise RuntimeError("Mysticeti signing block not found")
    text = text.replace(old, new, 1)
    old = '''        if !self.enabled {
            return Ok(BlockDigest::synthetic(block.round(), block.author()));
        }
        let digest = BlockDigest::new('''
    new = '''        let digest = BlockDigest::new('''
    if old not in text:
        raise RuntimeError("Mysticeti verification block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        '''        public_key.verify(block.signature(), digest.as_ref())?;
        Ok(digest.with_signature(block.signature()))''',
        '''        if self.enabled {
            public_key.verify(block.signature(), digest.as_ref())?;
        }
        Ok(digest.with_signature(block.signature()))''',
        1,
    )
    write(path, text)
