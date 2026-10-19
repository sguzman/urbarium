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
    const scale=Math.min(1.25,Math.max(.48,Math.min(w/820,h/610)));
    // The viewpoint is centered on the real spawn area, but elevated for
    // legibility; full Rust camera pose arrives with the real first frame.
    const project=(p,y=0)=>{
      const dx=p.x-camera.x,dz=p.z-camera.z;
      const right=dx*cs-dz*sn;
      const forward=dx*sn+dz*cs;
      return [w*.50+right*scale,h*.66-forward*scale*.57-y*scale*1.55];
    };
    const path=(points,y=0)=>{
      if(points.length<2)return false;
      const [x,z]=project(points[0],y);
      context.beginPath();context.moveTo(x,z);
      for(let i=1;i<points.length;i++){
        const [px,pz]=project(points[i],y);
        context.lineTo(px,pz);
      }
      return true;
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
      if(!path(s.points,0.04))return;
      const roads={motorway:15,trunk:12,primary:11,secondary:9,
        tertiary:8,residential:7,unclassified:6,service:4,
        living_street:5,pedestrian:3,footway:2,path:1.6,steps:1.4};
      const wide=roads[s.kind]||2;
      context.lineWidth=Math.max(1.2,wide*scale);
      context.strokeStyle=["footway","path","steps","pedestrian"].includes(s.kind)
        ? "#a7a597" : "#343b3e";
      context.stroke();
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
      const hgt=height;
      for(let i=0;i<ring.length;i++){
        const next=ring[(i+1)%ring.length];
        const a=project(ring[i]),b=project(next);
        const c=project(next,hgt),d=project(ring[i],hgt);
        context.fillStyle=i%2?"#81908e":"#748281";
        context.beginPath();context.moveTo(...a);context.lineTo(...b);
        context.lineTo(...c);context.lineTo(...d);context.closePath();context.fill();
      }
      const cap=ring.map(p=>project(p,hgt));
      context.fillStyle="#a4aca8";
      context.beginPath();context.moveTo(...cap[0]);
      for(const p of cap.slice(1))context.lineTo(...p);
      context.closePath();context.fill();
      buildings++;
    }
    const x=16,y=h-32;
    context.fillStyle="rgba(9,21,28,.86)";
    context.fillRect(x-8,y-13,Math.min(w-22,474),30);
    context.fillStyle="#c4dedb";
    context.font="12px system-ui,sans-serif";
    context.fillText("Nearby real streets + buildings · simplified early view",x,y+6);
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
      // We need neither full source tree nor a second CPU geometry copy
      // once the first compact canvas paint exists.
      data=null;
    }).catch(error=>{
      status.dataset.bootstrapPreview="unavailable";
      console.warn("Urbarium early geographic preview unavailable",error);
    });
})();
