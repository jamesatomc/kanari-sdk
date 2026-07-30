// Kanari Labs NFT Module - Universal Standard Version
module james::nft {
    use std::string::{String, utf8};
    use kanari_system::tx_context::TxContext;
    use kanari_system::tx_context;
    use kanari_system::transfer;
    use kanari_system::url as url;
    use kanari_system::object;
    use kanari_system::object::UID;
    use kanari_system::collection;
    use kanari_system::event;

    /// โครงสร้างสำหรับระบุความเป็นเจ้าของ (One-Time Witness)
    struct NFT has drop {}

    /// โครงสร้าง NFT แบบมาตรฐานครอบจักรวาล
    struct KariKid has key, store {
        /// 1. ID ของวัตถุ (UID)
        id: UID,
        /// 2. ชื่อของ NFT
        name: String,
        /// 3. URL ของรูปภาพ
        image_url: url::Url,
        /// 4. รายละเอียด (Description)
        description: String,
        /// 5. รายการหัวข้อคุณสมบัติ (เช่น ["Level", "Attack"])
        attribute_keys: vector<String>,
        /// 6. รายการค่าของคุณสมบัติ (เช่น ["10", "150"])
        attribute_values: vector<String>,

        // --- ฟิลด์เสริมที่อยู่นอกเหนือมาตรฐาน ---
        number: String,
        collection_id: address,
        creator: address,
    }

    /// Event สำหรับการ Mint
    struct MintLog has copy, drop {
        object_id: address,
        creator: address,
        collection_id: address,
    }

    const MAX_SUPPLY: u64 = 2000;

    /// ฟังก์ชันสร้างคอลเลกชัน (เรียกผ่าน setup หรือระบบ)
    /// 🚨 แก้ไข: เพิ่ม Banner และ Website เข้าไปให้ครบ 6 อาร์กิวเมนต์
    public fun init(_otw: NFT, ctx: &mut TxContext): (collection::Collection, collection::NftCap) {
        let name = b"KariKid Collection";
        let description = b"The first official standardized NFT on Kanari Network.";
        // Use a durable, directly-loadable image URL. A custom HTTPS, ipfs://,
        // or data:image/jpeg;base64,... URI can be set after setup as well.
        let banner = b"https://avatars.githubusercontent.com/u/127471673?s=800&v=4";
        let website = b"https://james-project.com";            // 🌐 ใส่ลิงก์เว็บที่นี่
        
        collection::create_collection(
            ctx, 
            name, 
            description, 
            banner, 
            website, 
            MAX_SUPPLY
        )
    }

    /// ฟังก์ชันตั้งค่าเริ่มต้น (Setup)
    /// 🚨 แก้ไข: เรียกฟังก์ชัน init ที่เราปรับปรุงแล้ว
    public entry fun setup(ctx: &mut TxContext) {
        let (coll, issuer) = init(NFT {}, ctx);
        let sender = tx_context::sender(ctx);
        transfer::public_transfer(issuer, sender);
        transfer::public_transfer(coll, sender);
    }

    /// Replaces collection artwork without recreating the collection. Only the
    /// original collection creator is authorized by collection::update_metadata.
    public entry fun update_collection_metadata(
        coll: &mut collection::Collection,
        banner: vector<u8>,
        website: vector<u8>,
        ctx: &mut TxContext,
    ) {
        collection::update_metadata(coll, banner, website, ctx);
    }

    /// ฟังก์ชัน Mint ตามมาตรฐานสากล
    public entry fun mint(
        cap: &mut collection::NftCap,
        name: vector<u8>,
        description: vector<u8>,
        url_bytes: vector<u8>,
        attribute_keys: vector<String>,
        attribute_values: vector<String>,
        number: vector<u8>,
        ctx: &mut TxContext
    ) {
        collection::consume_for_mint(cap);

        let sender = tx_context::sender(ctx);

        let nft = KariKid {
            id: object::new(ctx),
            name: utf8(name),
            image_url: url::new_unsafe_from_bytes(url_bytes),
            description: utf8(description),
            attribute_keys,
            attribute_values,
            number: utf8(number),
            collection_id: collection::cap_collection_id(cap),
            creator: sender,
        };

        event::emit(MintLog {
            object_id: object::uid_address(&nft.id),
            creator: sender,
            collection_id: collection::cap_collection_id(cap),
        });

        transfer::public_transfer(nft, sender);
    }

    /// การทำลาย NFT (Burn)
    public entry fun burn(
        cap: &mut collection::NftCap,
        nft: KariKid,
        _: &mut TxContext
    ) {
        collection::return_from_burn(cap);
        let KariKid { 
            id, 
            name: _, 
            image_url: _, 
            description: _, 
            attribute_keys: _, 
            attribute_values: _, 
            number: _, 
            collection_id: _, 
            creator: _ 
        } = nft;
        object::delete(id);
    }

    /// การโอน NFT
    public entry fun transfer(
        nft: KariKid, 
        recipient: address, 
        _: &mut TxContext
    ) {
        transfer::public_transfer(nft, recipient)
    }

    /// อัปเดตคุณสมบัติ (Attributes)
    public entry fun update_attributes(
        nft: &mut KariKid,
        attribute_keys: vector<String>,
        attribute_values: vector<String>,
        _: &mut TxContext
    ) {
        nft.attribute_keys = attribute_keys;
        nft.attribute_values = attribute_values;
    }
}
