use super::{CookedMap, MaterialInterface};

#[derive(Clone, Debug)]
pub struct CookedMaterial {
    pub interface: MaterialInterface,
    pub maps: Vec<CookedMap>,
    pub size: u32,
}
