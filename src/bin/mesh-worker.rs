//! Dedicated CPU-only WASM worker. No winit, WGPU, DOM, or canvas.
//! The same chunk_mesh::build_chunk_payload() is used by both execution paths.
#[cfg(target_arch="wasm32")]
use std::cell::RefCell;
#[cfg(target_arch="wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch="wasm32")]
use urbarium::{City,terrain::TerrainGrid,streaming::ChunkKey};
#[cfg(target_arch="wasm32")]
use urbarium::chunk_mesh::build_chunk_payload;
#[cfg(target_arch="wasm32")]
use urbarium::chunk_wire;

#[cfg(target_arch="wasm32")]
thread_local!{
    static DATA:RefCell<Option<(City,TerrainGrid)>>=const{RefCell::new(None)};
}
#[cfg(target_arch="wasm32")]
#[wasm_bindgen]
pub fn initialize_worker(city_json:&str,terrain_json:&str)->Result<(),JsValue>{
    let city:City=serde_json::from_str(city_json)
        .map_err(|e|JsValue::from_str(&format!("Worker city decode: {e}")))?;
    let terrain:TerrainGrid=serde_json::from_str(terrain_json)
        .map_err(|e|JsValue::from_str(&format!("Worker terrain decode: {e}")))?;
    terrain.validate(&city.region)
        .map_err(|e|JsValue::from_str(&format!("Worker terrain check: {e}")))?;
    if terrain.region_id!=city.region.id{
        return Err(JsValue::from_str("Worker data region mismatch"));
    }
    DATA.with(|cell|*cell.borrow_mut()=Some((city,terrain)));
    Ok(())
}
#[cfg(target_arch="wasm32")]
#[wasm_bindgen]
pub fn mesh_chunk(x:i32,z:i32)->Result<Vec<u8>,JsValue>{
    DATA.with(|cell|{
        let guard=cell.borrow();
        let (city,terrain)=guard.as_ref()
            .ok_or_else(||JsValue::from_str("Worker was not initialized"))?;
        let payload=build_chunk_payload(city,terrain,ChunkKey{x,z})
            .map_err(|e|JsValue::from_str(&e))?;
        chunk_wire::encode(ChunkKey{x,z},&payload)
            .map_err(|e|JsValue::from_str(&format!("Worker encode: {e}")))
    })
}
fn main(){}
