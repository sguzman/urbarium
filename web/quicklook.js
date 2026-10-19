/* A bounded, source-backed, camera-local first paint independent of WASM.
 * Only a low-cost geographic sketch is drawn while the main binary loads.
 * Nothing is fabricated: roads use imported polylines; building tops use
 * exterior footprints and source/level heights, with flat temporary ground.
 * The primary WebGPU/WebGL2 canvas fully replaces this sketch when ready.
 */
"use strict";
(() => {
  const status=document.getElementById("status");
  if(!status)return;
  const canvas=document.createElement("canvas");
  canvas.id="startup-preview";
  canvas.setAttribute("role","img");
  canvas.setAttribute("aria-label","Simplified source-backed nearby city while the 3D viewer initializes");
  canvas.style.pointerEvents="none";
  document.body.prepend(canvas);
  const context=canvas.getContext("2d",{alpha:false});
  if(!context)return;
  let finished=false;
  let data=null;
  let elapsedStart=performance.now();
  function clear(){
    const w=canvas.width,h=canvas.height;
    const sky=context.createLinearGradient(0,0,0,h);
    sky.addColorStop(0,"#152b36");
    sky.addColorStop(.5,"#415660");
    sky.addColorStop(.51,"#66716e");
    sky.addColorStop(1,"#57645e");
    context.fillStyle=sky; context.fillRect(0,0,w,h);
  }
  function resize(){
    canvas.width=Math.max(1,Math.min(1600,Math.round(innerWidth)));
    canvas.height=Math.max(1,Math.min(1000,Math.round(innerHeight)));
    if(data&&!finished)draw(data);
    else clear();
  }
  resize();
  addEventListener("resize",resize,{passive:true});
  const distance=(p,q)=>Math.hypot(p.x-q.x,p.z-q.z);
  function chooseCamera(city){
    const focus=document.documentElement.dataset.reviewFocus?.split(",").map(Number);
    const region=city.region;
    if(focus?.length===2 && focus.every(Number.isFinite)){
      const x=(focus[0]-region.origin[0])*6371008.8*
        Math.cos(region.origin[1]*Math.PI/180);
      const z=(focus[1]-region.origin[1])*6371008.8;
      if(Number.isFinite(x)&&Number.isFinite(z))return {x,z,yaw:Math.PI/4};
    }
    // Match Rust spawn_at_street's closest eligible segment to the
    // geographic origin. The exact Rust collision/terrain checks still win.
    let best=Infinity,camera={x:0,z:0,yaw:Math.PI/4};
    const allowed=new Set(["residential","living_street","service",
      "tertiary","secondary","primary","unclassified","pedestrian"]);
    for(const street of city.streets){
      if(!allowed.has(street.kind))continue;
      const path=street.path||[];
      for(let i=1;i<path.length;i++){
        const a=path[i-1],b=path[i];
        if(![a?.x,a?.z,b?.x,b?.z].every(Number.isFinite))continue;
        const length=distance(a,b);
        if(length<6||length>=1000)continue;
        const x=(a.x+b.x)/2,z=(a.z+b.z)/2,score=x*x+z*z;
        if(score<best){best=score;camera={x,z,yaw:Math.atan2(b.x-a.x,b.z-a.z)};}
      }
    }
    return camera;
  }
  function draw(city){
    if(finished)return;
    clear();
    const w=canvas.width,h=canvas.height;
    const camera=chooseCamera(city);
    const yaw=camera.yaw;
    const cs=Math.cos(yaw),sn=Math.sin(yaw);
    // The real Rust viewer starts looking horizontally from a 1.7 m walking
    // eye at 68° vertical FOV. Match that perspective. The old orthographic
    // elevated sketch made startup look like a flyover followed by a teleport.
    const eye=1.7,near=2.0;
    const focal=h/(2*Math.tan(68*Math.PI/360));
    const view=(p,y=0)=>{
      const dx=p.x-camera.x,dz=p.z-camera.z;
      return {right:dx*cs-dz*sn,depth:dx*sn+dz*cs,height:y-eye};
    };
    const screen=v=>[w*.5+v.right*focal/v.depth,h*.5-v.height*focal/v.depth];
    const clipNear=(a,b)=>{
      if(a.depth<near && b.depth<near)return null;
      if(a.depth<near||b.depth<near){
        const t=(near-a.depth)/(b.depth-a.depth);
        const cut={right:a.right+(b.right-a.right)*t,
          depth:near,height:a.height+(b.height-a.height)*t};
        if(a.depth<near)a=cut;else b=cut;
      }
      return [a,b];
    };
    const clipPolygon=vertices=>{
      const out=[];
      if(!vertices.length)return out;
      let previous=vertices[vertices.length-1];
      for(const current of vertices){
        const prevIn=previous.depth>=near,currIn=current.depth>=near;
        if(prevIn!==currIn){
          const t=(near-previous.depth)/(current.depth-previous.depth);
          out.push({right:previous.right+(current.right-previous.right)*t,
            depth:near,height:previous.height+(current.height-previous.height)*t});
        }
        if(currIn)out.push(current);
        previous=current;
      }
      return out;
    };
    const face=(vertices,color)=>{
      const clipped=clipPolygon(vertices);
      if(clipped.length<3)return;
      context.fillStyle=color;
      context.beginPath();context.moveTo(...screen(clipped[0]));
      for(const point of clipped.slice(1))context.lineTo(...screen(point));
      context.closePath();context.fill();
    };
    const local=p=>Number.isFinite(p?.x)&&Number.isFinite(p?.z) &&
      distance(p,camera)<530;
    const clipped=[];
    for(const s of city.streets){
      const points=(s.path||[]).filter(p=>Number.isFinite(p?.x)&&Number.isFinite(p?.z));
      if(points.length<2||!points.some(local))continue;
      clipped.push({points,kind:s.kind});
      if(clipped.length>=2800)break;
    }
    context.lineCap="round";context.lineJoin="round";
    const paintRoad=(s)=>{
      const roads={motorway:15,trunk:12,primary:11,secondary:9,
        tertiary:8,residential:7,unclassified:6,service:4,
        living_street:5,pedestrian:3,footway:2,path:1.6,steps:1.4};
      const wide=roads[s.kind]||2;
      context.strokeStyle=["footway","path","steps","pedestrian"].includes(s.kind)
        ? "#a7a597" : "#343b3e";
      for(let i=1;i<s.points.length;i++){
        const segment=clipNear(view(s.points[i-1],0.04),view(s.points[i],0.04));
        if(!segment)continue;
        const [a,b]=segment;
        context.lineWidth=Math.min(110,Math.max(1.2,
          wide*focal/Math.max(near,(a.depth+b.depth)*.5)));
        context.beginPath();context.moveTo(...screen(a));
        context.lineTo(...screen(b));context.stroke();
      }
    };
    for(const road of clipped){
      if(["footway","path","steps","pedestrian"].includes(road.kind))paintRoad(road);
    }
    for(const road of clipped){
      if(!["footway","path","steps","pedestrian"].includes(road.kind))paintRoad(road);
    }
    let buildings=0;
    const entries=[];
    for(const b of city.buildings){
      const exterior=b.polygons?.[0]?.rings?.[0];
      if(!Array.isArray(exterior)||exterior.length<3)continue;
      if(!exterior.some(local))continue;
      const ring=exterior.filter(p=>Number.isFinite(p?.x)&&Number.isFinite(p?.z));
      if(ring.length<3||ring.length>300)continue;
      const height=Number.isFinite(b.height_m)?b.height_m:
        Number.isFinite(b.levels)?b.levels*3.1:12;
      if(height<=0||height>700)continue;
      const c=ring.reduce((o,p)=>({x:o.x+p.x,z:o.z+p.z}),{x:0,z:0});
      c.x/=ring.length;c.z/=ring.length;
      const depth=(c.x-camera.x)*sn+(c.z-camera.z)*cs;
      entries.push({ring,height,depth});
      if(entries.length>=1100)break;
    }
    entries.sort((a,b)=>b.depth-a.depth);
    for(const {ring,height} of entries){
      if(!ring.some(p=>view(p).depth>near))continue;
      for(let i=0;i<ring.length;i++){
        const next=ring[(i+1)%ring.length];
        face([view(ring[i]),view(next),
          view(next,height),view(ring[i],height)],
          i%2?"#81908e":"#748281");
      }
      face(ring.map(p=>view(p,height)),"#a4aca8");
      buildings++;
    }
    const x=16,y=h-32;
    context.fillStyle="rgba(9,21,28,.86)";
    context.fillRect(x-8,y-13,Math.min(w-22,474),30);
    context.fillStyle="#c4dedb";
    context.font="12px system-ui,sans-serif";
    context.fillText("Nearby real streets + buildings · simplified early view",x,y+6);
    status.dataset.bootstrapPreviewCamera="street";
    status.dataset.bootstrapPreview="ready";
    status.dataset.bootstrapPreviewStreets=String(clipped.length);
    status.dataset.bootstrapPreviewBuildings=String(buildings);
    status.dataset.bootstrapPreviewMs=String(Math.round(performance.now()-elapsedStart));
    window.__urbariumBootstrapPreview={
      ready:true,streets:clipped.length,buildings,elapsedMs:performance.now()-elapsedStart
    };
  }
  const observer=new MutationObserver(()=>{
    if(status.dataset.renderStage!=="first-present"||finished)return;
    finished=true;observer.disconnect();
    requestAnimationFrame(()=>{
      requestAnimationFrame(()=>{canvas.remove();data=null;});
    });
  });
  observer.observe(status,{attributes:true,attributeFilter:["data-render-stage"]});
  // Start independently of Rust. The normal geographic fetch interceptor
  // keeps the same versioned payload available to the Rust loader later.
  const id=document.documentElement.dataset.activeRegion||"downtown-la";
  fetch("./assets/"+encodeURIComponent(id)+".json")
    .then(response=>{if(!response.ok)throw Error("HTTP "+response.status);return response.json();})
    .then(city=>{
      if(finished)return;
      if(!Array.isArray(city.streets)||!Array.isArray(city.buildings)||!city.region)
        throw Error("invalid geographic source");
      data=city;draw(city);
      // Retain the small source object only while the heavy renderer
      // is loading, so resizing cannot blank the early visible city.
      // It is released as soon as the real first frame is presented.
    }).catch(error=>{
      status.dataset.bootstrapPreview="unavailable";
      console.warn("Urbarium early geographic preview unavailable",error);
    });
})();
