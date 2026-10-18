//! Versioned compact transfer protocol for CPU-generated mesh chunks.
 //!
 //! The data is composed of GPU-native POD vertices, LE u32 indices, and
 //! fixed 12-byte draw descriptors. No mesh JSON is produced or parsed.
 //! This is a transport format, not an on-disk cache or a new mesher.
 //!
 //! Layout (all integers LE):
 //! magic "UBM1", cell x:i32, cell z:i32, vertex count:u32,
 //! index count:u32, batch count:u32, then:
 //! [GpuVertex; vertices], [u32; indices],
 //! [layer:u32, first_index:u32, count:u32; batches].
use crate::chunk_mesh::{ChunkMeshPayload, DrawBatch, GpuVertex, SceneLayer};
use crate::streaming::ChunkKey;
use std::mem::size_of;
const MAGIC:&[u8;4]=b"UBM1";
const HEADER_SIZE:usize=24;
const BATCH_SIZE:usize=12;
// These are hard safety limits, not normal sizes or visual culling thresholds.
const MAX_PACKET_BYTES:usize=200*1024*1024;
const MAX_BATCHES:usize=100_000;

fn layer_code(layer:SceneLayer)->Result<u32,String>{
    match layer {
        SceneLayer::Ground=>Ok(0),
        SceneLayer::Buildings=>Ok(1),
        SceneLayer::Road=>Ok(2),
        SceneLayer::ElevatedRoad=>Ok(3),
        SceneLayer::BridgeStructure=>Ok(4),
        SceneLayer::Walk=>Ok(5),
        SceneLayer::Steps=>Ok(6),
        SceneLayer::Rail=>Ok(7),
        SceneLayer::Marker(n) if n<8=>Ok(8+n),
        _=>Err("Unsupported marker category in worker packet".into()),
    }
}
fn layer_from(code:u32)->Result<SceneLayer,String>{
    match code{
        0=>Ok(SceneLayer::Ground),
        1=>Ok(SceneLayer::Buildings),
        2=>Ok(SceneLayer::Road),
        3=>Ok(SceneLayer::ElevatedRoad),
        4=>Ok(SceneLayer::BridgeStructure),
        5=>Ok(SceneLayer::Walk),
        6=>Ok(SceneLayer::Steps),
        7=>Ok(SceneLayer::Rail),
        8..=15=>Ok(SceneLayer::Marker(code-8)),
        _=>Err(format!("Invalid layer code: {code}")),
    }
}
fn read_u32(bytes:&[u8],offset:usize)->Result<u32,String>{
    let range=bytes.get(offset..offset+4)
        .ok_or_else(||"Truncated chunk mesh header or index".to_owned())?;
    Ok(u32::from_le_bytes(range.try_into().map_err(|_|"Truncated packet")?))
}
fn put_u32(out:&mut Vec<u8>,number:u32){
    out.extend_from_slice(&number.to_le_bytes());
}
fn shape(vertices:usize,indices:usize,batches:usize)->Result<usize,String>{
    if batches>MAX_BATCHES{return Err("Excessive worker draw batches".into());}
    let bytes=vertices.checked_mul(size_of::<GpuVertex>())
        .and_then(|n|indices.checked_mul(size_of::<u32>()).and_then(|i|n.checked_add(i)))
        .and_then(|n|batches.checked_mul(BATCH_SIZE).and_then(|b|n.checked_add(b)))
        .and_then(|n|n.checked_add(HEADER_SIZE))
        .ok_or_else(||"Worker packet size overflow".to_string())?;
    if bytes>MAX_PACKET_BYTES{return Err("Worker packet size exceeds safety limit".into());}
    Ok(bytes)
}
fn validate(payload:&ChunkMeshPayload)->Result<(),String>{
    let length=payload.vertices.len();
    if payload.vertices.iter().any(|v|
        !v.position.iter().chain(v.normal.iter()).chain(v.color.iter())
            .chain(v.category_color.iter()).chain(std::iter::once(&v.category_id))
            .all(|n|n.is_finite())){
        return Err("Non-finite worker vertex".into());
    }
    if payload.indices.iter().any(|i|(*i as usize)>=length) {
        return Err("Worker mesh index outside vertex array".into());
    }
    for batch in &payload.batches{
        layer_code(batch.layer)?;
        let end=batch.first_index.checked_add(batch.count)
            .ok_or_else(||"Worker draw batch overflow".to_owned())?;
        if end as usize>payload.indices.len() || batch.count%3!=0 {
            return Err("Worker draw batch outside mesh index range".into());
        }
    }
    Ok(())
}
/// Pack a chunk into a single transferable Uint8Array. The Web Worker posts
/// its owned backing buffer with a transfer list (not a structured JSON copy).
pub fn encode(key:ChunkKey,payload:&ChunkMeshPayload)->Result<Vec<u8>,String>{
    validate(payload)?;
    let size=shape(payload.vertices.len(),payload.indices.len(),payload.batches.len())?;
    let to_u32=|n:usize|u32::try_from(n).map_err(|_|"Worker count exceeds u32".to_string());
    let mut out=Vec::with_capacity(size);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&key.x.to_le_bytes());
    out.extend_from_slice(&key.z.to_le_bytes());
    put_u32(&mut out,to_u32(payload.vertices.len())?);
    put_u32(&mut out,to_u32(payload.indices.len())?);
    put_u32(&mut out,to_u32(payload.batches.len())?);
    out.extend_from_slice(bytemuck::cast_slice(&payload.vertices));
    for index in &payload.indices{put_u32(&mut out,*index);}
    for batch in &payload.batches{
        put_u32(&mut out,layer_code(batch.layer)?);
        put_u32(&mut out,batch.first_index);
        put_u32(&mut out,batch.count);
    }
    debug_assert_eq!(out.len(),size);
    Ok(out)
}
/// Reject version mismatches, truncation, bad counts/indices, and invalid
/// geometry before handing the decoded result to the existing GPU uploader.
pub fn decode(bytes:&[u8])->Result<(ChunkKey,ChunkMeshPayload),String>{
    if bytes.len()<HEADER_SIZE || bytes.get(..4)!=Some(MAGIC.as_slice()){
        return Err("Invalid or unsupported worker mesh packet".into());
    }
    let key=ChunkKey{x:read_u32(bytes,4)? as i32,z:read_u32(bytes,8)? as i32};
    let nv=read_u32(bytes,12)? as usize;
    let ni=read_u32(bytes,16)? as usize;
    let nb=read_u32(bytes,20)? as usize;
    let expected=shape(nv,ni,nb)?;
    if bytes.len()!=expected{return Err("Worker mesh packet length mismatch".into());}
    let vertex_end=HEADER_SIZE+nv*size_of::<GpuVertex>();
    let mut vertices=Vec::with_capacity(nv);
    for v in bytes[HEADER_SIZE..vertex_end].chunks_exact(size_of::<GpuVertex>()){
        vertices.push(bytemuck::pod_read_unaligned::<GpuVertex>(v));
    }
    let indices_end=vertex_end+ni*size_of::<u32>();
    let mut indices=Vec::with_capacity(ni);
    for i in bytes[vertex_end..indices_end].chunks_exact(4){
        indices.push(u32::from_le_bytes(i.try_into()
            .map_err(|_|"Worker truncated index")?));
    }
    let mut batches=Vec::with_capacity(nb);
    for b in bytes[indices_end..].chunks_exact(BATCH_SIZE){
        batches.push(DrawBatch{
            layer:layer_from(read_u32(b,0)?)?,
            first_index:read_u32(b,4)?,
            count:read_u32(b,8)?,
        });
    }
    let payload=ChunkMeshPayload{vertices,indices,batches};
    validate(&payload)?;
    Ok((key,payload))
}
#[cfg(test)]
mod tests{
    use super::*;
    fn example()->ChunkMeshPayload{
        let vertex=GpuVertex{
            position:[10.5,30.25,-45.5],normal:[0.0,1.0,0.0],
            color:[0.2,0.4,0.6],category_color:[0.1,0.3,0.5],category_id:6.0,
        };
        ChunkMeshPayload{
            vertices:vec![vertex;3],
            indices:vec![0,1,2],
            batches:vec![
                DrawBatch{layer:SceneLayer::Buildings,first_index:0,count:3},
                DrawBatch{layer:SceneLayer::Marker(7),first_index:0,count:3}
            ]
        }
    }
    #[test]
    fn binary_roundtrip_preserves_gpu_bytes_indexes_and_marker_layers(){
        let payload=example();let key=ChunkKey{x:-4,z:5};
        let packet=encode(key,&payload).unwrap();
        assert_eq!(&packet[0..4],MAGIC);
        assert_eq!(packet.len(),HEADER_SIZE+3*size_of::<GpuVertex>()+12+24);
        let (decoded_key,decoded)=decode(&packet).unwrap();
        assert_eq!(key,decoded_key);
        assert_eq!(bytemuck::cast_slice::<GpuVertex,u8>(&payload.vertices),
                   bytemuck::cast_slice::<GpuVertex,u8>(&decoded.vertices));
        assert_eq!(payload.indices,decoded.indices);
        assert_eq!(decoded.batches[0].layer,SceneLayer::Buildings);
        assert_eq!(decoded.batches[1].layer,SceneLayer::Marker(7));
        assert_eq!(decoded.batches[1].first_index,0);
    }
    #[test]
    fn empty_mesh_is_valid_and_needs_no_special_case(){
        let payload=ChunkMeshPayload{vertices:vec![],indices:vec![],batches:vec![]};
        let key=ChunkKey{x:1,z:-1};
        let packet=encode(key,&payload).unwrap();
        assert_eq!(packet.len(),HEADER_SIZE);
        assert!(decode(&packet).unwrap().1.vertices.is_empty());
    }
    #[test]
    fn malformed_packets_fail_closed(){
        let data=encode(ChunkKey{x:2,z:3},&example()).unwrap();
        for len in 0..data.len(){
            assert!(decode(&data[..len]).is_err(),"truncated length {len}");
        }
        let mut corrupt=data.clone();corrupt[0]=b'X';
        assert!(decode(&corrupt).is_err());
        corrupt=data.clone();corrupt[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&corrupt).is_err());
        corrupt=data.clone();
        let last_batch=corrupt.len()-12;
        corrupt[last_batch..last_batch+4].copy_from_slice(&999u32.to_le_bytes());
        assert!(decode(&corrupt).is_err());
        corrupt=data.clone();
        corrupt[HEADER_SIZE..HEADER_SIZE+4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(&corrupt).is_err());
        corrupt=data;
        let first_index=HEADER_SIZE+3*size_of::<GpuVertex>();
        corrupt[first_index..first_index+4].copy_from_slice(&999u32.to_le_bytes());
        assert!(decode(&corrupt).is_err());
    }
}
