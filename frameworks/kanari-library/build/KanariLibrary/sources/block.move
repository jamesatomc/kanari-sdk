module kanari_library::block {

    use moveos_std::signer;
    use moveos_std::object;
    use moveos_std::hash;
    use moveos_std::bcs;

    /// Only admin can create blocks
    const ErrorAlreadyExists: u64 = 1;

    struct BlockHeader has key, store {
        version: u32,
        prev_block_hash: vector<u8>,
        merkle_root: vector<u8>,
        time: u64,
        nonce: u64,
    }

    struct Block has key, store {
        header: BlockHeader,
        transactions: vector<vector<u8>>,
    }

    fun compute_hash(header: &BlockHeader): vector<u8> {
        // Concatenate all header fields for hashing
        let data = std::vector::empty<u8>();

        // Append prev_block_hash
        std::vector::append(&mut data, header.prev_block_hash);

        // Append merkle root
        std::vector::append(&mut data, header.merkle_root);

        // Convert time and nonce to bytes using BCS and append
        let time_bytes = bcs::to_bytes(&header.time);
        let nonce_bytes = bcs::to_bytes(&header.nonce);

        std::vector::append(&mut data, time_bytes);
        std::vector::append(&mut data, nonce_bytes);

        // Return SHA256 hash of concatenated data
        hash::sha3_256(data)
    }

    public entry fun create_block(admin: &signer, prev_hash: vector<u8>, merkle_root: vector<u8>, time: u64, nonce: u64, transactions: vector<vector<u8>>) {
        // Only admin can create blocks
        assert!(signer::address_of(admin) == @kanari_library, ErrorAlreadyExists);

        let header = BlockHeader {
            version: 1,
            prev_block_hash: prev_hash,
            merkle_root: merkle_root,
            time: time,
            nonce: nonce,
        };

        let block = Block {
            header: header,
            transactions: transactions,
        };

        // Compute hash
        let _hash = compute_hash(&block.header);

        // Store the block (assuming there's a way to do this in the framework)
        // Placeholder code
        let block_obj = object::new_named_object(block);
        object::transfer(block_obj, @kanari_library);
    }
}
