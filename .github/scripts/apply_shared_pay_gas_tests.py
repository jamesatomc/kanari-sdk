from pathlib import Path

path = Path("crates/kanari-types/src/object_transaction.rs")
text = path.read_text()
old = '''    #[test]
    fn rejects_gas_object_reused_as_pay_coin() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let duplicate = object_ref("0x20", 3);
        let transaction = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::Pay {
                coins: vec![duplicate],
                recipient: AccountAddress::from_hex_literal("0x2").unwrap(),
                amount: 10,
            },
            GasData {
                payment: vec![duplicate],
                owner: sender,
                price: 1,
                budget: 100,
            },
            TransactionExpiration::None,
        );
        assert!(transaction.is_err());
    }
'''
new = '''    #[test]
    fn allows_sender_owned_pay_coin_to_fund_gas() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let shared = object_ref("0x20", 3);
        let transaction = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::Pay {
                coins: vec![shared],
                recipient: AccountAddress::from_hex_literal("0x2").unwrap(),
                amount: 10,
            },
            GasData {
                payment: vec![shared],
                owner: sender,
                price: 1,
                budget: 100,
            },
            TransactionExpiration::None,
        );
        assert!(transaction.is_ok());
    }

    #[test]
    fn rejects_sponsored_gas_reused_as_sender_pay_coin() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let sponsor = AccountAddress::from_hex_literal("0x3").unwrap();
        let shared = object_ref("0x20", 3);
        let transaction = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::Pay {
                coins: vec![shared],
                recipient: AccountAddress::from_hex_literal("0x2").unwrap(),
                amount: 10,
            },
            GasData {
                payment: vec![shared],
                owner: sponsor,
                price: 1,
                budget: 100,
            },
            TransactionExpiration::None,
        );
        assert!(transaction.is_err());
    }
'''
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise RuntimeError("shared pay/gas test marker was not found")
path.write_text(text)
