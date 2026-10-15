//! Source-qualified elevated transportation mesh pass.
//!
//! The bridge flag and positive level denote relative vertical stacking,
//! NOT a surveyed altitude. Deck clearance, thickness, barriers and piers
//! here are deliberately marked visual estimates, not engineering assets.
//! Ordinary motorways remain on terrain unless grade separation is tagged.
use crate::geometry::{Mesh, Vertex};
use crate::roads::{BridgeSpan, RoadScene, SurfaceKind, RIBBON_CROSS_STEPS, ribbon};

/// One render-only approximate roadway level. Not a measured bridge height.
const VISUAL_CLEARANCE_M:f32=5.8;
const DECK_THICKNESS_M:f32=0.65;
const BARRIER_HEIGHT_M:f32=0.85;
const PIER_SPACING_M:f64=30.0;

fn append(target:&mut Mesh, part:Mesh)->Result<(),String>{
    let base=u32::try_from(target.vertices.len()).map_err(|_|"bridge vertices exceed u32")?;
    if part.indices.iter().any(|i|(*i as usize)>=part.vertices.len() ||
        i.checked_add(base).is_none()) {
        return Err("bridge mesh index exceeds u32".into());
    }
    target.vertices.extend(part.vertices);
    target.indices.extend(part.indices.into_iter().map(|i|i+base));
    Ok(())
}
fn quad(mesh:&mut Mesh,a:[f32;3],b:[f32;3],c:[f32;3],d:[f32;3]){
    let start=mesh.vertices.len() as u32;
    let v=[b[0]-a[0],b[1]-a[1],b[2]-a[2]];
    let w=[c[0]-a[0],c[1]-a[1],c[2]-a[2]];
    let n=[v[1]*w[2]-v[2]*w[1],v[2]*w[0]-v[0]*w[2],
        v[0]*w[1]-v[1]*w[0]];
    let length=(n[0]*n[0]+n[1]*n[1]+n[2]*n[2]).sqrt().max(1e-9);
    let normal=[n[0]/length,n[1]/length,n[2]/length];
    for position in [a,b,c,d] {
        mesh.vertices.push(Vertex{position,normal});
    }
    mesh.indices.extend([start,start+1,start+2,start,start+2,start+3]);
}
fn box_pier(mesh:&mut Mesh,x:f32,z:f32,bottom:f32,top:f32){
    if top<=bottom+0.5{return;}
    let r=0.65;
    let a=[x-r,bottom,z-r];
    let b=[x+r,bottom,z-r];
    let c=[x+r,bottom,z+r];
    let d=[x-r,bottom,z+r];
    let e=[x-r,top,z-r];
    let f=[x+r,top,z-r];
    let g=[x+r,top,z+r];
    let h=[x-r,top,z+r];
    quad(mesh,a,b,f,e);
    quad(mesh,b,c,g,f);
    quad(mesh,c,d,h,g);
    quad(mesh,d,a,e,h);
    quad(mesh,e,f,g,h);
}

/// Regenerate independent elevated meshes with an external geographic terrain
/// sampler. This runs with a flat sampler for export, then with DEM data in
/// the browser, ensuring no bridge deck is painted over a ground-level road.
///
/// Source paths have X east / Z north in meters. The 5.8m tier clearance is
/// a *visual heuristic*; it must never be confused with a measured clearance.
pub fn build_bridge_layers(
    scene:&mut RoadScene,
    sample:impl Fn(f64,f64)->f32,
)->Result<(),String>{
    for kind in [SurfaceKind::ElevatedRoad,SurfaceKind::BridgeFascia,
                 SurfaceKind::BridgeSupports,SurfaceKind::BridgeRails] {
        scene.layers.remove(&kind);
    }
    let mut decks=Mesh::default();
    let mut fascia=Mesh::default();
    let mut supports=Mesh::default();
    let mut rails=Mesh::default();
    for span in &scene.bridge_spans {
        let Some(mut deck)=ribbon(&span.path,span.width_m,0.0)? else {continue;};
        let rows=deck.vertices.len()/(RIBBON_CROSS_STEPS+1);
        if rows<2 {continue;}
        let stride=RIBBON_CROSS_STEPS+1;
        let mut stations=Vec::with_capacity(rows);
        let mut distance=0f64;
        for i in 0..rows {
            let center=deck.vertices[i*stride+stride/2].position;
            if i>0 {
                let previous=deck.vertices[(i-1)*stride+stride/2].position;
                distance+=(center[0]-previous[0]).hypot(center[2]-previous[2]) as f64;
            }
            stations.push(distance);
            let terrain=sample(center[0] as f64,center[2] as f64);
            let top=terrain+VISUAL_CLEARANCE_M*span.relative_level as f32+0.24;
            for j in 0..stride {
                deck.vertices[i*stride+j].position[1]=top;
            }
        }
        for i in 0..rows-1 {
            let current=i*stride;
            let next=(i+1)*stride;
            let l0=deck.vertices[current].position;
            let l1=deck.vertices[next].position;
            let r0=deck.vertices[current+stride-1].position;
            let r1=deck.vertices[next+stride-1].position;
            let lower=|mut v:[f32;3]| {v[1]-=DECK_THICKNESS_M;v};
            // Side fascia + actual soffit rather than a floating paper ribbon.
            quad(&mut fascia,l0,l1,lower(l1),lower(l0));
            quad(&mut fascia,r1,r0,lower(r0),lower(r1));
            quad(&mut fascia,lower(r0),lower(l0),lower(l1),lower(r1));
            let raised=|mut v:[f32;3]| {v[1]+=BARRIER_HEIGHT_M;v};
            quad(&mut rails,l0,l1,raised(l1),raised(l0));
            quad(&mut rails,r1,r0,raised(r0),raised(r1));
        }
        // The columns are illustrative vertical members under open decks.
        // They are not surveyed pier positions and not physical collision.
        if distance>12.0 {
            let mut target=PIER_SPACING_M*0.5;
            for i in 1..rows-1 {
                if stations[i]<target || stations[i]>distance-6.0 {continue;}
                let center=deck.vertices[i*stride+stride/2].position;
                let ground=sample(center[0] as f64,center[2] as f64)+0.20;
                box_pier(&mut supports,center[0],center[2],ground,
                    center[1]-DECK_THICKNESS_M);
                target+=PIER_SPACING_M;
            }
        }
        append(&mut decks,deck)?;
    }
    for (kind,mesh) in [
        (SurfaceKind::ElevatedRoad,decks),
        (SurfaceKind::BridgeFascia,fascia),
        (SurfaceKind::BridgeSupports,supports),
        (SurfaceKind::BridgeRails,rails),
    ] {
        if !mesh.indices.is_empty() {scene.layers.insert(kind,mesh);}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{City,Point,Region,Street};
    use std::collections::BTreeMap;
    fn tagged(kind:&str,tags:&[(&str,&str)])->Street {
        Street{
            source_id:kind.into(),name:None,kind:kind.into(),
            path:vec![Point{x:0.,z:0.},Point{x:0.,z:95.}],
            osm_node_ids:vec![],tags:tags.iter().map(|(a,b)|(a.to_string(),b.to_string())).collect(),
        }
    }
    fn scene(roads:Vec<Street>)->RoadScene {
        let city=City{
            schema_version:1,
            region:Region{id:"test".into(),name:"test".into(),
                origin:[0.,0.],bbox:[-0.02,-0.02,0.02,0.02]},
            provenance:vec![],streets:roads,buildings:vec![],
        };
        crate::roads::mesh_streets(&city).unwrap().0
    }
    #[test]
    fn tagged_bridge_has_real_deck_thickness_barriers_and_piers(){
        let roads=scene(vec![
            tagged("motorway",&[("bridge","yes"),("layer","1")]),
        ]);
        assert_eq!(roads.bridge_spans.len(),1);
        let deck=&roads.layers[&SurfaceKind::ElevatedRoad];
        assert!(deck.vertices.iter().all(|v|v.position[1]>5.8));
        assert!(!roads.layers[&SurfaceKind::BridgeFascia].indices.is_empty());
        assert!(!roads.layers[&SurfaceKind::BridgeSupports].indices.is_empty());
        assert!(!roads.layers[&SurfaceKind::BridgeRails].indices.is_empty());
        for mesh in roads.layers.values() {
            assert!(mesh.indices.iter().all(|&i|i as usize<mesh.vertices.len()));
        }
    }
    #[test]
    fn motorway_without_bridge_source_remains_on_the_ground(){
        let roads=scene(vec![tagged("motorway",&[])]);
        assert_eq!(roads.bridge_spans.len(),0);
        assert!(!roads.layers.contains_key(&SurfaceKind::ElevatedRoad));
        assert!(roads.layers.contains_key(&SurfaceKind::Road));
    }
    #[test]
    fn terrain_sampler_drives_deck_and_support_ground_levels(){
        let mut roads=scene(vec![tagged("primary",&[("bridge","yes"),("layer","2")])]);
        build_bridge_layers(&mut roads,|_,z|(z as f32)*0.1).unwrap();
        let deck=&roads.layers[&SurfaceKind::ElevatedRoad];
        let first=deck.vertices[2].position;
        let last=deck.vertices[deck.vertices.len()-3].position;
        assert!((first[1]-11.84).abs()<0.01);
        assert!((last[1]-(11.84+9.5)).abs()<0.02);
    }
    #[test]
    fn tunnel_is_not_mistaken_for_an_overpass(){
        let roads=scene(vec![tagged("primary",&[
            ("bridge","yes"),("tunnel","yes"),("layer","-1")
        ])]);
        assert!(roads.bridge_spans.is_empty());
    }
}
