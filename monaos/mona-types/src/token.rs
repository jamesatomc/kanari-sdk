use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::collections::HashMap;
use std::string::String as StdString;

use crate::address::Address;
use crate::balance::Balance;
use crate::coin::Coin;
use crate::object::{ID, UID};

/// The Token system - a closed loop token with configurable policy.
/// Corresponds to `kanari_framework::token::Token<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token<T> {
    pub id: UID,
    pub balance: Balance<T>,
}

/// Token policy that defines rules for token operations.
/// Corresponds to `kanari_framework::token::TokenPolicy<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenPolicy<T> {
    pub id: UID,
    pub spent_balance: Balance<T>,
    pub rules: Vec<StdString>,
    _phantom: PhantomData<T>,
}

/// Capability for managing token policy.
/// Corresponds to `kanari_framework::token::TokenPolicyCap<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenPolicyCap<T> {
    pub id: UID,
    pub policy_id: ID,
    _phantom: PhantomData<T>,
}

/// Request for performing an action on a token.
/// Corresponds to `kanari_framework::token::ActionRequest<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest<T> {
    pub policy_id: ID,
    pub action: StdString,
    pub spent_balance: Option<Balance<T>>,
    pub approvals: Vec<StdString>,
    pub metadata: HashMap<StdString, Vec<u8>>,
    _phantom: PhantomData<T>,
}

/// Different types of token actions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenAction {
    Transfer,
    Spend,
    ToCoin,
    FromCoin,
}

impl TokenAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenAction::Transfer => "transfer",
            TokenAction::Spend => "spend", 
            TokenAction::ToCoin => "to_coin",
            TokenAction::FromCoin => "from_coin",
        }
    }
}

impl<T> Token<T> {
    /// Create a new token from balance
    pub fn from_balance(balance: Balance<T>, id: UID) -> Self {
        Self { id, balance }
    }

    /// Get the token's value
    pub fn value(&self) -> u64 {
        self.balance.value()
    }

    /// Get immutable reference to the balance
    pub fn balance(&self) -> &Balance<T> {
        &self.balance
    }

    /// Get mutable reference to the balance
    pub fn balance_mut(&mut self) -> &mut Balance<T> {
        &mut self.balance
    }

    /// Convert token into balance
    pub fn into_balance(self) -> Balance<T> {
        self.balance
    }

    /// Split the token into two tokens
    pub fn split(&mut self, value: u64, new_id: UID) -> Result<Token<T>, TokenError> {
        let new_balance = self.balance.split(value)
            .map_err(|_| TokenError::BalanceTooLow)?;
        
        Ok(Token::from_balance(new_balance, new_id))
    }

    /// Join another token into this one
    pub fn join(&mut self, other: Token<T>) {
        self.balance.join(other.balance);
    }

    /// Check if the token has zero value
    pub fn is_zero(&self) -> bool {
        self.balance.is_zero()
    }

    /// Destroy a zero-value token
    pub fn destroy_zero(self) -> Result<(), TokenError> {
        if !self.is_zero() {
            return Err(TokenError::NotZero);
        }
        Ok(())
    }
}

impl<T> TokenPolicy<T> {
    /// Create a new token policy
    pub fn new(id: UID, rules: Vec<StdString>) -> Self {
        Self {
            id,
            spent_balance: Balance::zero(),
            rules,
            _phantom: PhantomData,
        }
    }

    /// Check if an action is allowed by the policy
    pub fn is_action_allowed(&self, action: &str) -> bool {
        self.rules.iter().any(|rule| rule == action)
    }

    /// Add a rule to the policy
    pub fn add_rule(&mut self, rule: StdString) {
        if !self.rules.contains(&rule) {
            self.rules.push(rule);
        }
    }

    /// Remove a rule from the policy
    pub fn remove_rule(&mut self, rule: &str) {
        self.rules.retain(|r| r != rule);
    }

    /// Get the spent balance
    pub fn spent_balance(&self) -> &Balance<T> {
        &self.spent_balance
    }

    /// Withdraw spent balance
    pub fn withdraw_spent_balance(&mut self) -> Balance<T> {
        self.spent_balance.withdraw_all()
    }
}

impl<T> TokenPolicyCap<T> {
    /// Create a new token policy capability
    pub fn new(id: UID, policy_id: ID) -> Self {
        Self {
            id,
            policy_id,
            _phantom: PhantomData,
        }
    }

    /// Get the policy ID this capability controls
    pub fn policy_id(&self) -> ID {
        self.policy_id
    }
}

impl<T> ActionRequest<T> {
    /// Create a new action request
    pub fn new(
        policy_id: ID,
        action: TokenAction,
        spent_balance: Option<Balance<T>>,
    ) -> Self {
        Self {
            policy_id,
            action: action.as_str().to_string(),
            spent_balance,
            approvals: Vec::new(),
            metadata: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    /// Add approval to the request
    pub fn add_approval(&mut self, approval: StdString) {
        if !self.approvals.contains(&approval) {
            self.approvals.push(approval);
        }
    }

    /// Check if the request has a specific approval
    pub fn has_approval(&self, approval: &str) -> bool {
        self.approvals.iter().any(|a| a == approval)
    }

    /// Add metadata to the request
    pub fn add_metadata(&mut self, key: StdString, value: Vec<u8>) {
        self.metadata.insert(key, value);
    }

    /// Get metadata from the request
    pub fn get_metadata(&self, key: &str) -> Option<&Vec<u8>> {
        self.metadata.get(key)
    }

    /// Get the action string
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Get the policy ID
    pub fn policy_id(&self) -> ID {
        self.policy_id
    }

    /// Take the spent balance
    pub fn take_spent_balance(&mut self) -> Option<Balance<T>> {
        self.spent_balance.take()
    }
}

/// Token operations
pub struct TokenOperations;

impl TokenOperations {
    /// Create a new token from coin
    pub fn from_coin<T>(
        coin: Coin<T>,
        policy: &TokenPolicy<T>,
        new_id: UID,
    ) -> Result<(Token<T>, ActionRequest<T>), TokenError> {
        if !policy.is_action_allowed(TokenAction::FromCoin.as_str()) {
            return Err(TokenError::UnknownAction);
        }

        let balance = coin.into_balance();
        let token = Token::from_balance(balance, new_id);
        let request = ActionRequest::new(
            policy.id.to_id(),
            TokenAction::FromCoin,
            None,
        );

        Ok((token, request))
    }

    /// Convert token to coin
    pub fn to_coin<T>(
        token: Token<T>,
        policy: &TokenPolicy<T>,
        coin_id: crate::coin::ObjectId,
    ) -> Result<(Coin<T>, ActionRequest<T>), TokenError> {
        if !policy.is_action_allowed(TokenAction::ToCoin.as_str()) {
            return Err(TokenError::UnknownAction);
        }

        let balance = token.into_balance();
        let coin = Coin::from_balance(balance, coin_id);
        let request = ActionRequest::new(
            policy.id.to_id(),
            TokenAction::ToCoin,
            None,
        );

        Ok((coin, request))
    }    /// Transfer a token
    pub fn transfer<T>(
        _token: Token<T>,
        policy: &TokenPolicy<T>,
        recipient: Address,
    ) -> Result<ActionRequest<T>, TokenError> {
        if !policy.is_action_allowed(TokenAction::Transfer.as_str()) {
            return Err(TokenError::UnknownAction);
        }

        let mut request = ActionRequest::new(
            policy.id.to_id(),
            TokenAction::Transfer,
            None,
        );

        // Add recipient to metadata
        request.add_metadata("recipient".to_string(), recipient.to_bytes().to_vec());

        Ok(request)
    }

    /// Spend a token
    pub fn spend<T>(
        token: Token<T>,
        policy: &TokenPolicy<T>,
    ) -> Result<ActionRequest<T>, TokenError> {
        if !policy.is_action_allowed(TokenAction::Spend.as_str()) {
            return Err(TokenError::UnknownAction);
        }

        let balance = token.into_balance();
        let request = ActionRequest::new(
            policy.id.to_id(),
            TokenAction::Spend,
            Some(balance),
        );

        Ok(request)
    }

    /// Confirm an action request
    pub fn confirm_request<T>(
        policy: &mut TokenPolicy<T>,
        request: ActionRequest<T>,
        cap: &TokenPolicyCap<T>,
    ) -> Result<(), TokenError> {
        if cap.policy_id != policy.id.to_id() {
            return Err(TokenError::NotAuthorized);
        }

        if let Some(spent_balance) = request.spent_balance {
            policy.spent_balance.join(spent_balance);
        }

        Ok(())
    }
}

/// Token operation errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("The action is not allowed in the policy")]
    UnknownAction,
    #[error("The rule was not approved")]
    NotApproved,
    #[error("Admin action with wrong capability")]
    NotAuthorized,
    #[error("Balance is too low to perform the action")]
    BalanceTooLow,
    #[error("Balance is not zero")]
    NotZero,
    #[error("Cannot consume balance")]
    CantConsumeBalance,
    #[error("Rule is trying to access missing config")]
    NoConfig,
    #[error("Use immutable confirm function")]
    UseImmutableConfirm,
}

/// Token error constants matching Move constants
pub mod error_constants {
    /// The action is not allowed (defined) in the policy.
    pub const E_UNKNOWN_ACTION: u64 = 0;
    /// The rule was not approved.
    pub const E_NOT_APPROVED: u64 = 1;
    /// Trying to perform an admin action with a wrong cap.
    pub const E_NOT_AUTHORIZED: u64 = 2;
    /// The balance is too low to perform the action.
    pub const E_BALANCE_TOO_LOW: u64 = 3;
    /// The balance is not zero.
    pub const E_NOT_ZERO: u64 = 4;
    /// The balance is not zero when trying to confirm with `TransferPolicyCap`.
    pub const E_CANT_CONSUME_BALANCE: u64 = 5;
    /// Rule is trying to access a missing config (with type).
    pub const E_NO_CONFIG: u64 = 6;
    /// Using `confirm_request_mut` without `spent_balance`.
    pub const E_USE_IMMUTABLE_CONFIRM: u64 = 7;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_creation() {
        let balance = Balance::with_value(100);
        let id = UID::default();
        let token = Token::from_balance(balance, id);
        
        assert_eq!(token.value(), 100);
        assert!(!token.is_zero());
    }

    #[test]
    fn test_token_split() {
        let balance = Balance::with_value(100);
        let id1 = UID::default();
        let id2 = UID::default();
        let mut token = Token::from_balance(balance, id1);
        
        let split_token = token.split(30, id2).unwrap();
        
        assert_eq!(token.value(), 70);
        assert_eq!(split_token.value(), 30);
    }

    #[test]
    fn test_token_policy() {
        let id = UID::default();
        let rules = vec!["transfer".to_string(), "spend".to_string()];
        let policy = TokenPolicy::new(id, rules);
        
        assert!(policy.is_action_allowed("transfer"));
        assert!(policy.is_action_allowed("spend"));
        assert!(!policy.is_action_allowed("mint"));
    }

    #[test]
    fn test_action_request() {
        let policy_id = ID::default();
        let mut request = ActionRequest::new(
            policy_id,
            TokenAction::Transfer,
            None,
        );
        
        request.add_approval("validator1".to_string());
        request.add_metadata("recipient".to_string(), vec![1, 2, 3, 4]);
        
        assert!(request.has_approval("validator1"));
        assert_eq!(request.get_metadata("recipient"), Some(&vec![1, 2, 3, 4]));
        assert_eq!(request.action(), "transfer");
    }
}
