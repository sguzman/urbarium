//! Source-qualified elevated transportation mesh pass.
//!
//! The bridge flag and positive level denote relative vertical stacking,
//! NOT a surveyed altitude. Deck clearance, thickness, barriers and piers
//! here are deliberately marked visual estimates, not engineering assets.
//! Ordinary motorways remain on terrain unless grade separation is tagged.
use crate::geometry::{Mesh, Vertex};
use crate::roads::{BridgeSpan, RoadScene, SurfaceKind, RIBBON_CROSS_STEPS, ribbon, motorway_edge_stripes};
use crate::Point;

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
/// Retire unfinished elevated geometry in the normal viewer while preserving
/// the actual source centerlines as ordinary terrain-draped road surfaces.
/// No bridge deck, supports, barriers or GPU bridge batches are allocated.
/// The experimental preview retains the separate grade mesh pipeline.
pub fn flatten_bridge_spans(scene:&mut RoadScene)->Result<(),String>{
    for kind in [
        SurfaceKind::ElevatedRoad,SurfaceKind::BridgeFascia,
        SurfaceKind::BridgeSupports,SurfaceKind::BridgeRails,
        SurfaceKind::BridgeStripe,
    ]{
        scene.layers.remove(&kind);
    }
    for span in &scene.bridge_spans {
        let Some(strip)=ribbon(&span.path,span.width_m,0.24)? else{continue;};
        let motorway=span.kind.starts_with("motorway")||
            span.kind.starts_with("trunk");
        let kind=if motorway{SurfaceKind::Motorway}else{SurfaceKind::Road};
        if motorway {
            let paint=motorway_edge_stripes(&strip,0.035);
            append(scene.layers.entry(SurfaceKind::MotorwayStripe)
                .or_default(),paint)?;
        }
        append(scene.layers.entry(kind).or_default(),strip)?;
    }
    Ok(())
}

const APPROACH_M:f64=65.0;
const MATCH_RADIUS_M:f64=1.6;

/// Road tags alone never authorize connecting two crossing centerlines.
/// A ramp must attach to an at-grade, heading-compatible ROAD ENDPOINT.
fn same_corridor(a:&str,b:&str)->bool {
    a==b || (a.starts_with("motorway")&&b.starts_with("motorway")) ||
    (a.starts_with("trunk")&&b.starts_with("trunk"))
}
fn separation(a:Point,b:Point)->f64 {(a.x-b.x).hypot(a.z-b.z)}
fn unit(p:Point,q:Point)->Option<[f64;2]> {
    let x=q.x-p.x;let z=q.z-p.z;let d=x.hypot(z);
    (d>0.01).then_some([x/d,z/d])
}
fn smoothstep(t:f64)->f32{
    let t=t.clamp(0.0,1.0) as f32;
    t*t*(3.0-2.0*t)
}
/// Trace at-grade linework away from an actual bridge endpoint. Follow only
/// smooth, true endpoint joins and never bridge an arbitrary 2D crossing.
fn approach_path(scene:&RoadScene,span:&BridgeSpan,start:bool)->Option<Vec<Point>>{
    let bridge=&span.path;
    let joined=if start{bridge[0]}else{*bridge.last()?};
    let inside=if start{bridge[1]}else{bridge[bridge.len()-2]};
    let mut back=unit(joined,inside)?;
    let mut tip=joined;
    let mut visited=std::collections::BTreeSet::new();
    let mut grown=vec![joined];
    let mut length=0.0;
    for _ in 0..12{
        let mut match_road=None;
        let mut best=-2.0;
        for (index,road) in scene.approach_roads.iter().enumerate(){
            if visited.contains(&index)||road.path.len()<2||
                !same_corridor(&span.kind,&road.kind){continue;}
            for reverse in [false,true]{
                let (p,q)=if reverse{
                    (*road.path.last().unwrap(),road.path[road.path.len()-2])
                }else{(road.path[0],road.path[1])};
                if separation(p,tip)>MATCH_RADIUS_M {continue;}
                let Some(v)=unit(p,q) else{continue;};
                let dot=back[0]*v[0]+back[1]*v[1];
                // Competing junction branches must not become phantom ramps.
                if dot > -0.70 {continue;}
                let score=-dot-separation(p,tip)*0.10;
                if score>best{best=score;match_road=Some((index,reverse));}
            }
        }
        let Some((index,reverse))=match_road else{break;};
        visited.insert(index);
        let road=&scene.approach_roads[index];
        let path:Vec<Point>=if reverse{road.path.iter().rev().copied().collect()}
            else{road.path.clone()};
        for next in path.iter().copied().skip(1){
            let d=separation(tip,next);
            if d<0.01 {continue;}
            if length+d>=APPROACH_M {
                let t=(APPROACH_M-length)/d;
                grown.push(Point{x:tip.x+(next.x-tip.x)*t,z:tip.z+(next.z-tip.z)*t});
                length=APPROACH_M;
                break;
            }
            grown.push(next);length+=d;tip=next;
        }
        if length>=APPROACH_M {break;}
        if grown.len()<2 {break;}
        tip=*grown.last().unwrap();
        back=unit(tip,grown[grown.len()-2])?;
    }
    // Short fragments would create implausibly steep ramps. Taper on the
    // bridge instead, rather than inventing continuation.
    (length>=20.0).then_some(grown)
}
fn joined_bridge(scene:&RoadScene,index:usize,start:bool)->bool{
    let span=&scene.bridge_spans[index];
    let p=if start{span.path[0]}else{*span.path.last().unwrap()};
    scene.bridge_spans.iter().enumerate().any(|(other_index,other)|{
        other_index!=index && other.relative_level==span.relative_level &&
        same_corridor(&other.kind,&span.kind) &&
        (separation(other.path[0],p)<MATCH_RADIUS_M||
        separation(*other.path.last().unwrap(),p)<MATCH_RADIUS_M)
    })
}
#[derive(Clone,Copy)]
enum Profile {
    ApproachUp,
    ApproachDown,
    Deck{start_taper:bool,end_taper:bool},
}
fn add_profile(
    path:&[Point],width:f64,level:u8,profile:Profile,is_motorway:bool,
    sample:&impl Fn(f64,f64)->f32,
    decks:&mut Mesh,fascia:&mut Mesh,supports:&mut Mesh,rails:&mut Mesh,
    stripe:&mut Mesh,
)->Result<(),String>{
    let Some(mut deck)=ribbon(path,width,0.0)? else{return Ok(());};
    let stride=RIBBON_CROSS_STEPS+1;
    let rows=deck.vertices.len()/stride;
    if rows<2{return Ok(());}
    let mut station=vec![0.0f64;rows];
    for i in 1..rows {
        let a=deck.vertices[(i-1)*stride+stride/2].position;
        let b=deck.vertices[i*stride+stride/2].position;
        station[i]=station[i-1]+(b[0]-a[0]).hypot(b[2]-a[2]) as f64;
    }
    let total=station[rows-1].max(0.001);
    for i in 0..rows{
        let center=deck.vertices[i*stride+stride/2].position;
        let ratio=station[i]/total;
        let factor=match profile{
            Profile::ApproachUp=>smoothstep(ratio),
            Profile::ApproachDown=>smoothstep(1.0-ratio),
            Profile::Deck{start_taper,end_taper}=>{
                let taper=(total*0.42).min(25.0).max(0.001);
                let start=if start_taper{smoothstep(station[i]/taper)}else{1.0};
                let end=if end_taper{smoothstep((total-station[i])/taper)}else{1.0};
                start.min(end)
            }
        };
        let height=sample(center[0] as f64,center[2] as f64)+0.24+
            VISUAL_CLEARANCE_M*level as f32*factor;
        for j in 0..stride{deck.vertices[i*stride+j].position[1]=height;}
    }
    for i in 0..rows-1{
        let current=i*stride;let next=(i+1)*stride;
        let l0=deck.vertices[current].position;
        let l1=deck.vertices[next].position;
        let r0=deck.vertices[current+stride-1].position;
        let r1=deck.vertices[next+stride-1].position;
        let lower=|mut v:[f32;3]|{v[1]-=DECK_THICKNESS_M;v};
        quad(fascia,l0,l1,lower(l1),lower(l0));
        quad(fascia,r1,r0,lower(r0),lower(r1));
        quad(fascia,lower(r0),lower(l0),lower(l1),lower(r1));
        let raised=|mut v:[f32;3]|{v[1]+=BARRIER_HEIGHT_M;v};
        quad(rails,l0,l1,raised(l1),raised(l0));
        quad(rails,r1,r0,raised(r0),raised(r1));
    }
    if total>12.0 {
        let mut target=PIER_SPACING_M*0.5;
        for i in 1..rows-1{
            if station[i]<target||station[i]>total-6.0{continue;}
            let p=deck.vertices[i*stride+stride/2].position;
            let ground=sample(p[0] as f64,p[2] as f64)+0.20;
            if p[1]-DECK_THICKNESS_M-ground>=2.5 {
                box_pier(supports,p[0],p[2],ground,p[1]-DECK_THICKNESS_M);
            }
            target+=PIER_SPACING_M;
        }
    }
    if is_motorway {
        append(stripe,motorway_edge_stripes(&deck,0.04))?;
    }
    append(decks,deck)
}

/// The generated geometry is a visually smooth APPROXIMATION tied to source
/// bridge tags. Full freeway topology outside the city's bounding box cannot
/// be synthesized from clipped centerlines.
pub fn build_bridge_layers(
    scene:&mut RoadScene,
    sample:impl Fn(f64,f64)->f32,
)->Result<(),String>{
    for kind in [SurfaceKind::ElevatedRoad,SurfaceKind::BridgeFascia,
                 SurfaceKind::BridgeSupports,SurfaceKind::BridgeRails,
                 SurfaceKind::BridgeStripe] {
        scene.layers.remove(&kind);
    }
    let (mut decks,mut fascia,mut supports,mut rails,mut stripe)=
        (Mesh::default(),Mesh::default(),Mesh::default(),Mesh::default(),Mesh::default());
    for (index,span) in scene.bridge_spans.iter().enumerate(){
        if span.path.len()<2 {continue;}
        let is_motorway=span.kind.starts_with("motorway")||span.kind.starts_with("trunk");
        let start_join=joined_bridge(scene,index,true);
        let end_join=joined_bridge(scene,index,false);
        let before=if !start_join&&!span.clipped_start {
            approach_path(scene,span,true)
        }else{None};
        let after=if !end_join&&!span.clipped_end {
            approach_path(scene,span,false)
        }else{None};
        if let Some(path)=&before{
            let reverse:Vec<Point>=path.iter().rev().copied().collect();
            add_profile(&reverse,span.width_m,span.relative_level,
                Profile::ApproachUp,is_motorway,&sample,
                &mut decks,&mut fascia,&mut supports,&mut rails,&mut stripe)?;
        }
        add_profile(&span.path,span.width_m,span.relative_level,
            Profile::Deck{
                start_taper:before.is_none()&&!start_join&&!span.clipped_start,
                end_taper:after.is_none()&&!end_join&&!span.clipped_end,
            },is_motorway,&sample,&mut decks,&mut fascia,&mut supports,&mut rails,&mut stripe)?;
        if let Some(path)=&after{
            add_profile(path,span.width_m,span.relative_level,
                Profile::ApproachDown,is_motorway,&sample,
                &mut decks,&mut fascia,&mut supports,&mut rails,&mut stripe)?;
        }
    }
    for (kind,mesh) in [
        (SurfaceKind::ElevatedRoad,decks),
        (SurfaceKind::BridgeFascia,fascia),
        (SurfaceKind::BridgeSupports,supports),
        (SurfaceKind::BridgeRails,rails),
        (SurfaceKind::BridgeStripe,stripe),
    ]{
        if !mesh.indices.is_empty(){scene.layers.insert(kind,mesh);}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{City,Point,Region,Street};
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
    fn retired_bridge_mode_keeps_centerline_without_raised_deck(){
        let mut scene=scene(vec![
            tagged("motorway",&[("bridge","yes"),("layer","1")]),
        ]);
        assert!(!scene.bridge_spans.is_empty());
        flatten_bridge_spans(&mut scene).unwrap();
        for kind in [SurfaceKind::ElevatedRoad,SurfaceKind::BridgeFascia,
            SurfaceKind::BridgeSupports,SurfaceKind::BridgeRails,
            SurfaceKind::BridgeStripe] {
            assert!(!scene.layers.contains_key(&kind));
        }
        let mesh=&scene.layers[&SurfaceKind::Motorway];
        assert!(!mesh.indices.is_empty());
        assert!(mesh.vertices.iter().all(|v|(v.position[1]-0.24).abs()<0.001));
        assert!(!scene.layers[&SurfaceKind::MotorwayStripe].indices.is_empty());
    }
    #[test]
    fn tagged_bridge_has_real_deck_thickness_barriers_and_piers(){
        let roads=scene(vec![
            tagged("motorway",&[("bridge","yes"),("layer","1")]),
        ]);
        assert_eq!(roads.bridge_spans.len(),1);
        let deck=&roads.layers[&SurfaceKind::ElevatedRoad];
        assert!(deck.vertices.iter().any(|v|v.position[1]>5.8));
        assert!((deck.vertices[2].position[1]-0.24).abs()<0.01,
            "Unconnected interior bridge end must taper to ground, not form a cliff");
        assert!(!roads.layers[&SurfaceKind::BridgeFascia].indices.is_empty());
        assert!(!roads.layers[&SurfaceKind::BridgeSupports].indices.is_empty());
        assert!(!roads.layers[&SurfaceKind::BridgeRails].indices.is_empty());
        for mesh in roads.layers.values() {
            assert!(mesh.indices.iter().all(|&i|(i as usize)<mesh.vertices.len()));
        }
    }
    #[test]
    fn motorway_without_bridge_source_remains_on_the_ground(){
        let roads=scene(vec![tagged("motorway",&[])]);
        assert_eq!(roads.bridge_spans.len(),0);
        assert!(!roads.layers.contains_key(&SurfaceKind::ElevatedRoad));
        assert!(roads.layers.contains_key(&SurfaceKind::Motorway));
        assert!(roads.layers.contains_key(&SurfaceKind::MotorwayStripe));
    }
    #[test]
    fn terrain_sampler_drives_deck_and_support_ground_levels(){
        let mut roads=scene(vec![tagged("primary",&[("bridge","yes"),("layer","2")])]);
        build_bridge_layers(&mut roads,|_,z|(z as f32)*0.1).unwrap();
        let deck=&roads.layers[&SurfaceKind::ElevatedRoad];
        let first=deck.vertices[2].position;
        let last=deck.vertices[deck.vertices.len()-3].position;
        assert!((first[1]-0.24).abs()<0.01);
        assert!((last[1]-(0.24+9.5)).abs()<0.02);
        let midpoint=deck.vertices[deck.vertices.len()/2].position;
        assert!(midpoint[1]>11.84, "Bridge center must rise above sampled terrain");
    }
    #[test]
    fn connected_bridge_builds_gradual_approaches_from_real_at_grade_endpoints(){
        let mut before=tagged("motorway",&[]);
        before.path=vec![Point{x:0.,z:-75.},Point{x:0.,z:0.}];
        let bridge=tagged("motorway",&[("bridge","yes")]);
        let mut after=tagged("motorway",&[]);
        after.path=vec![Point{x:0.,z:95.},Point{x:0.,z:170.}];
        let road_scene=scene(vec![before,bridge,after]);
        assert_eq!(road_scene.bridge_spans.len(),1);
        assert_eq!(road_scene.approach_roads.len(),2);
        let deck=&road_scene.layers[&SurfaceKind::ElevatedRoad];
        let bridge_only_rows=(95f64/3.0).ceil() as usize+1;
        assert!(deck.vertices.len()>(bridge_only_rows+1)*RIBBON_CROSS_STEPS,
            "Ramp geometry must extend beyond tagged bridge centerline");
        let first=deck.vertices[RIBBON_CROSS_STEPS/2].position;
        let last=deck.vertices[deck.vertices.len()-3].position;
        assert!((first[1]-0.24).abs()<0.1);
        assert!((last[1]-0.24).abs()<0.1);
        assert!(deck.vertices.iter().any(|v|v.position[1]>6.0));
        assert!(deck.vertices.iter().all(|v|v.position[1]>=0.23));
    }
    #[test]
    fn no_invented_bridge_connection_at_crossing_or_wrong_heading(){
        let mut crossing=tagged("primary",&[]);
        crossing.path=vec![Point{x:-35.,z:0.},Point{x:35.,z:0.}];
        let bridge=tagged("motorway",&[("bridge","yes")]);
        let road_scene=scene(vec![crossing,bridge]);
        assert!(approach_path(&road_scene,&road_scene.bridge_spans[0],true).is_none());
    }
    #[test]
    fn clipped_pilot_boundary_does_not_artificially_sink_bridge(){
        let mut road_scene=scene(vec![tagged("motorway",&[("bridge","yes")])]);
        road_scene.bridge_spans[0].clipped_start=true;
        road_scene.bridge_spans[0].clipped_end=true;
        build_bridge_layers(&mut road_scene,|_,_|0.0).unwrap();
        let deck=&road_scene.layers[&SurfaceKind::ElevatedRoad];
        assert!((deck.vertices[2].position[1]-6.04).abs()<0.02);
        assert!((deck.vertices[deck.vertices.len()-3].position[1]-6.04).abs()<0.02);
    }
    #[test]
    fn tunnel_is_not_mistaken_for_an_overpass(){
        let roads=scene(vec![tagged("primary",&[
            ("bridge","yes"),("tunnel","yes"),("layer","-1")
        ])]);
        assert!(roads.bridge_spans.is_empty());
    }
}
