use mona_types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_operations() {
        // Test balance creation and operations
        let mut balance = balance::Balance::<u64>::new(100);
        assert_eq!(balance.value(), 100);

        // Test split
        let split_balance = balance.split(30).unwrap();
        assert_eq!(balance.value(), 70);
        assert_eq!(split_balance.value(), 30);

        // Test join
        balance.join(split_balance);
        assert_eq!(balance.value(), 100);
    }

    #[test]
    fn test_coin_operations() {
        // Test coin creation
        let coin = coin::Coin::<u64>::new(500);
        assert_eq!(coin.value(), 500);

        // Test coin metadata
        let metadata = coin::CoinMetadata::<u64>::new(
            "Test Coin".to_string(),
            "TST".to_string(),
            8,
            "A test coin".to_string(),
            None,
        );
        assert_eq!(metadata.name(), "Test Coin");
        assert_eq!(metadata.symbol(), "TST");
        assert_eq!(metadata.decimals(), 8);
    }    #[test]
    fn test_object_system() {
        // Test ID creation
        let addr = address::Address::from_hex("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let id = object::ID::new(addr);
        assert_eq!(id.address().as_bytes().len(), 20);

        // Test UID creation
        let uid = object::UID::new(id);
        assert_eq!(uid.id().address().as_bytes().len(), 20);
    }    #[test]
    fn test_tx_context() {
        let sender = address::Address::from_hex("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let tx_context = tx_context::TxContext::new(sender, 100, 10, vec![0; 32], 1);
        
        assert_eq!(tx_context.sender(), &sender);
        assert_eq!(tx_context.epoch(), 100);
        assert_eq!(tx_context.epoch_timestamp_ms(), 10);

        // Test fresh address generation
        let fresh_addr = tx_context.fresh_object_address();
        assert!(fresh_addr.as_bytes().len() > 0);
    }

    #[test]
    fn test_event_system() {
        // Create an event emitter
        let emitter = event::MemoryEventEmitter::new();

        // Create a test event
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
        struct TestEvent {
            message: String,
            value: u64,
        }

        let test_event = TestEvent {
            message: "Hello, World!".to_string(),
            value: 42,
        };

        // Emit the event
        emitter.emit(test_event.clone());

        // Check events were captured
        let events = emitter.get_events();
        assert_eq!(events.len(), 1);
        
        let event_data = &events[0];
        assert!(event_data.event_type().contains("TestEvent"));
        
        // Test deserialization
        let deserialized: TestEvent = event_data.deserialize().unwrap();
        assert_eq!(deserialized, test_event);
    }

    #[test]
    fn test_kari_operations() {
        // Test KARI token operations
        let kari_amount = kari::KARI::new(1000);
        assert_eq!(kari_amount.value(), 1000);

        // Test conversion to KA
        let ka_amount = kari_amount.to_ka();
        assert_eq!(ka_amount.value(), 1); // 1000 KARI = 1 KA

        // Test formatting
        let formatted = kari_amount.format_with_symbol();
        assert!(formatted.contains("KARI"));

        // Test parsing
        let parsed = kari::KARI::from_str("500").unwrap();
        assert_eq!(parsed.value(), 500);
    }    #[test]
    fn test_transfer_types() {
        use transfer::*;

        #[derive(Debug, Clone)]
        struct TestObject {
            id: u64,
        }

        impl TransferableObject for TestObject {
            fn object_id(&self) -> object::ID {
                object::ID::new(address::Address::zero())
            }
        }

        // Test receiving type
        let test_obj = TestObject { id: 42 };
        let addr = address::Address::zero();
        let receiving = Receiving::new(test_obj, addr);
        
        // In a real implementation, this would work with the object system
        assert!(receiving.to_string().contains("Receiving"));
    }

    #[test]
    fn test_token_policy_system() {
        // Test token policy creation
        let mut policy = token::TokenPolicy::<u64>::new();
        
        // Allow transfer action
        policy.allow_action("transfer");
        assert!(policy.is_action_allowed("transfer"));
        assert!(!policy.is_action_allowed("burn"));

        // Test token creation would require more complex setup
        // This is a basic structure test
        let token = token::Token::<u64>::new(100, vec![1; 32]);
        assert_eq!(token.value(), 100);
    }

    #[test]
    fn test_gas_operations() {
        // Test gas fee calculations
        let gas_fee = gas::GasFee::new(1000, 5);
        assert_eq!(gas_fee.budget(), 1000);
        assert_eq!(gas_fee.price(), 5);
        assert_eq!(gas_fee.max_cost(), 5000);

        // Test gas consumption
        let mut gas_budget = gas::GasBudget::new(1000);
        let consumption_result = gas_budget.consume(300);
        assert!(consumption_result.is_ok());
        assert_eq!(gas_budget.remaining(), 700);

        // Test insufficient gas
        let insufficient_result = gas_budget.consume(800);
        assert!(insufficient_result.is_err());
    }

    #[test]
    fn test_address_operations() {
        // Test address creation from hex
        let addr = address::Address::from_hex("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(addr.as_bytes().len(), 20);

        // Test address formatting
        let hex_str = addr.to_hex();
        assert!(hex_str.starts_with("0x"));
        assert_eq!(hex_str.len(), 42); // 0x + 40 hex chars

        // Test zero address
        let zero_addr = address::Address::zero();
        assert_eq!(zero_addr.to_hex(), "0x0000000000000000000000000000000000000000");
    }
}
