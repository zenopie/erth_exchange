
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Order {
    pub user: Option<Addr>,
    pub input_remaining: Uint128,
}

pub static BUY_ORDERS: Keymap<Uint128, Vec<Order>> = Keymap::new(b"buy_orders");

pub static SELL_ORDERS: Keymap<Uint128, Vec<Order>> = Keymap::new(b"sell_orders");