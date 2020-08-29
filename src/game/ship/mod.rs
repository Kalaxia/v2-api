pub mod model;
pub mod queue;
pub mod squadron;

#[derive(Deserialize)]
pub struct ShipQuantityData {
    pub formation: FleetFormation,
    pub category: ShipModelCategory,
    pub quantity: usize
}