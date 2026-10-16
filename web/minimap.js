/* Geographic Downtown LA locator, 60Hz camera marker + world-space POI labels.
 * Background geometry is rasterized only on zoom or cache recenter. The camera
 * pointer and world labels track the compact Rust camera pose each RAF.
 * Landmarks are coordinate-anchored labels, NOT reconstructed architecture.
 */
(() => {
  "use strict";
  const canvas = document.getElementById("minimap-canvas");
  const ctx = canvas?.getContext("2d", { alpha: false });
  if (!ctx) return;
  const size=260, bufferSize=780;
  const dpr=Math.min(2,window.devicePixelRatio||1);
  canvas.width=Math.round(size*dpr);
  canvas.height=Math.round(size*dpr);
  ctx.setTransform(dpr,0,0,dpr,0,0);
  const buffer=document.createElement("canvas");
  buffer.width=Math.round(bufferSize*dpr);
  buffer.height=Math.round(bufferSize*dpr);
  const bc=buffer.getContext("2d",{alpha:false});
  bc.setTransform(dpr,0,0,dpr,0,0);
  const status=document.getElementById("status");
  const scaleLabel=document.getElementById("map-scale");
  const coordsLabel=document.getElementById("map-coordinates");
  const layer=document.getElementById("world-landmarks");
  const radius=(Math.PI/180)*6371008.8;
  const kinds=new Set(["motorway","motorway_link","trunk","trunk_link",
    "primary","primary_link","secondary","secondary_link","tertiary",
    "tertiary_link","residential","living_street","service","unclassified","track"]);
  const colors={road:"#668497",path:"#b6b9a5",rail:"#80a4ac",
    steps:"#b4aa91",buildings:"#425e68",ground:"#273f3d"};
  const palette=window.urbariumLandmarkPalette||{};
  const kindNames={civic:"Civic",legal:"Legal",library:"Library",
    culture:"Culture",park:"Park",railway:"Transit",market:"Market",
    historic:"Historic"};
  const allMask=(1<<Object.keys(palette).length)-1;
  const legend=document.getElementById("landmark-legend");
  const state={city:null,terrain:null,landmarks:[],filterMask:allMask,width:650,cache:false,
    cx:0,cz:0,frame:0,lastPose:null,mapRedraws:0};
  function project(lon,lat,region) {
    return {x:(lon-region.origin[0])*radius*Math.cos(region.origin[1]*Math.PI/180),
      z:(lat-region.origin[1])*radius};
  }
  function pose() {
    const packed=status?.getAttribute("data-camera-pose");
    if(packed) {
      const a=packed.split(",").map(Number);
      if(a.length===5&&a.every(Number.isFinite))
        return {x:a[0],y:a[1],z:a[2],yaw:a[3],pitch:a[4]};
    }
    const read=(name)=>Number(status?.getAttribute(name)||0);
    return {x:read("data-camera-x"),y:read("data-camera-y"),
      z:read("data-camera-z"),yaw:read("data-camera-yaw"),pitch:0};
  }
  function zoom(factor) {
    state.width=Math.max(100,Math.min(2500,state.width*factor));
    state.cache=false;
  }
  document.getElementById("map-zoom-in")?.addEventListener("click",()=>zoom(.65));
  document.getElementById("map-zoom-out")?.addEventListener("click",()=>zoom(1/.65));
  canvas.addEventListener("wheel",(e)=>{e.preventDefault();zoom(e.deltaY>0?1.18:1/1.18);},
    {passive:false});
  ["click","mousedown","mouseup"].forEach(event=>
    canvas.addEventListener(event,e=>e.stopPropagation()));

  function selected(kind) {
    const bit=Object.keys(palette).indexOf(kind);
    return bit<0||(state.filterMask&(1<<bit))!==0;
  }
  function buildingKind(building) {
    // Color the actual Overture footprint only when its landmark anchor is
    // INSIDE the polygon (and outside courtyard holes). No fake matching to
    // an unrelated nearest building.
    for (const lm of state.landmarks) {
      if(!selected(lm.kind))continue;
      for(const polygon of building.polygons||[]) {
        const rings=polygon.rings||[];
        if(rings.length && withinRing(lm.x,lm.z,rings[0]) &&
            !rings.slice(1).some(r=>withinRing(lm.x,lm.z,r)))
          return lm.kind;
      }
    }
    return null;
  }
  function rebuild(x,z) {
    const city=state.city,ppm=size/state.width;
    state.cx=x;state.cz=z;state.cache=true;state.mapRedraws++;
    const xy=p=>[bufferSize/2+(p.x-x)*ppm,bufferSize/2-(p.z-z)*ppm];
    bc.fillStyle="#101d27";
    bc.fillRect(0,0,bufferSize,bufferSize);
    const sw=xy(project(city.region.bbox[0],city.region.bbox[1],city.region));
    const ne=xy(project(city.region.bbox[2],city.region.bbox[3],city.region));
    bc.fillStyle=colors.ground;
    bc.fillRect(sw[0],ne[1],ne[0]-sw[0],sw[1]-ne[1]);
    for(const building of city.buildings) {
      const kind=buildingKind(building);
      bc.fillStyle=palette[kind]||colors.buildings;
      for(const polygon of building.polygons||[]) {
        bc.beginPath();
        for(const ring of polygon.rings||[]) {
          ring.forEach((p,i)=>{
            const pos=xy(p);
            if(i===0)bc.moveTo(pos[0],pos[1]);
            else bc.lineTo(pos[0],pos[1]);
          });
          bc.closePath();
        }
        bc.fill("evenodd");
      }
    }
    for(const street of city.streets) {
      if(!street.path||street.path.length<2)continue;
      bc.beginPath();
      bc.strokeStyle=kinds.has(street.kind)?colors.road:
        street.kind==="steps"?colors.steps:
        ["rail","tram","light_rail","funicular"].includes(street.kind)?
          colors.rail:colors.path;
      bc.lineWidth=kinds.has(street.kind)?1.7:.9;
      bc.lineCap="round";
      street.path.forEach((p,i)=>{
        const pos=xy(p);
        if(i===0)bc.moveTo(pos[0],pos[1]);
        else bc.lineTo(pos[0],pos[1]);
      });
      bc.stroke();
    }
    // Experimental bridge geometry is invisible by default. Cyan source-
    // flagged sections only appear when the explicit prototype is enabled.
    if(document.documentElement.dataset.bridgePreview==="true"){
    bc.save();
    bc.lineWidth=3.0;
    bc.lineCap="round";
    bc.strokeStyle="#78d8fb";
    for(const street of city.streets){
      if(!kinds.has(street.kind)||!street.path||street.path.length<2)continue;
      const elevated=street.tags?.bridge==="yes"||
        Number(street.tags?.layer||0)>0;
      if(!elevated||street.tags?.tunnel==="yes")continue;
      bc.beginPath();
      street.path.forEach((p,i)=>{
        const pt=xy(p);
        if(i)bc.lineTo(pt[0],pt[1]);else bc.moveTo(pt[0],pt[1]);
      });
      bc.stroke();
    }
    bc.restore();
    }
    bc.strokeStyle="#81979b";
    bc.lineWidth=1;
    bc.strokeRect(sw[0],ne[1],ne[0]-sw[0],sw[1]-ne[1]);
  }

  function terrainHeight(x,z) {
    const t=state.terrain,c=state.city;
    if(!t||!c||t.width<2||t.height<2)return 0;
    const sw=project(c.region.bbox[0],c.region.bbox[1],c.region);
    const ne=project(c.region.bbox[2],c.region.bbox[3],c.region);
    const fx=Math.max(0,Math.min(t.width-1,(x-sw.x)/(ne.x-sw.x)*(t.width-1)));
    const fz=Math.max(0,Math.min(t.height-1,(z-sw.z)/(ne.z-sw.z)*(t.height-1)));
    const col=Math.min(t.width-2,Math.floor(fx)),row=Math.min(t.height-2,Math.floor(fz));
    const tx=fx-col,tz=fz-row;
    const at=(ix,iz)=>t.heights_m[iz*t.width+ix];
    const swy=at(col,row),sey=at(col+1,row),nwy=at(col,row+1),
      ney=at(col+1,row+1);
    return tx+tz<=1?swy+(sey-swy)*tx+(nwy-swy)*tz:
      ney+(nwy-ney)*(1-tx)+(sey-ney)*(1-tz);
  }
  function withinRing(x,z,ring) {
    let hit=false,j=ring.length-1;
    for(let i=0;i<ring.length;j=i++) {
      const a=ring[i],b=ring[j];
      if((a.z>z)!==(b.z>z)&&x<(b.x-a.x)*(z-a.z)/(b.z-a.z)+a.x)
        hit=!hit;
    }
    return hit;
  }
  function landmarkHeight(mark) {
    let height=12;
    for(const b of state.city.buildings) {
      const h=Number.isFinite(b.height_m)?b.height_m:
        Number.isFinite(b.levels)?b.levels*3.1:0;
      for(const poly of b.polygons||[]) {
        const rings=poly.rings||[];
        if(rings.length&&withinRing(mark.x,mark.z,rings[0])&&
          !rings.slice(1).some(r=>withinRing(mark.x,mark.z,r))) {
          height=Math.max(height,h+5);
          return terrainHeight(mark.x,mark.z)+height;
        }
      }
    }
    return terrainHeight(mark.x,mark.z)+height;
  }

  function installLandmarks(doc) {
    if(!doc||doc.region_id!==state.city.region.id||
      !Array.isArray(doc.landmarks))return;
    state.landmarks=doc.landmarks
      .filter(item=>item.coordinates?.length===2 &&
        item.coordinates.every(Number.isFinite) &&
        item.coordinates[0]>=state.city.region.bbox[0] &&
        item.coordinates[0]<=state.city.region.bbox[2] &&
        item.coordinates[1]>=state.city.region.bbox[1] &&
        item.coordinates[1]<=state.city.region.bbox[3])
      .map(item=>{
        const pos=project(item.coordinates[0],item.coordinates[1],state.city.region);
        const mark={...item,x:pos.x,z:pos.z};
        mark.y=landmarkHeight(mark);
        if(layer) {
          const el=document.createElement("span");
          el.className="world-landmark";
          el.dataset.kind=mark.kind;
          el.textContent=mark.short||mark.name;
          el.title=mark.name+" · "+(kindNames[mark.kind]||mark.kind)+
            " · geographic landmark label (not a custom 3D model)";
          layer.appendChild(el);
          mark.el=el;
        }
        return mark;
      });
    canvas.setAttribute("data-landmark-count",String(state.landmarks.length));
    // Expose grounded classifications so the legend reports actual counts.
    const counts=Object.create(null);
    for(const mark of state.landmarks) counts[mark.kind]=(counts[mark.kind]||0)+1;
    canvas.setAttribute("data-legal-count",String(counts.legal||0));
    window.dispatchEvent(new CustomEvent("urbarium-landmarks-ready",
      {detail:{counts}}));
    state.cache=false;
    applyCategoryFilter();
  }
  function applyCategoryFilter() {
    const value=Number(legend?.dataset.visibleMask);
    state.filterMask=Number.isInteger(value)?value&allMask:allMask;
    state.cache=false;
    for(const mark of state.landmarks) {
      if(!mark.el)continue;
      mark.el.style.setProperty("--lm-color",palette[mark.kind]||"#ffd77d");
      mark.el.textContent=(mark.short||mark.name)+
        " · "+(kindNames[mark.kind]||mark.kind);
      mark.el.dataset.filterVisible=String(selected(mark.kind));
      if(!selected(mark.kind))mark.el.hidden=true;
    }
    canvas.dataset.categoryColors="true";
    canvas.dataset.categoryMask=String(state.filterMask);
    canvas.dataset.filterVisibleCount=String(state.landmarks.filter(
      mark=>selected(mark.kind)).length);
  }
  window.addEventListener("urbarium-landmark-filter-changed",applyCategoryFilter);
  applyCategoryFilter();

  function drawLandmarks(p,ppm) {
    ctx.save();
    ctx.font="11px system-ui, sans-serif";
    let drawn=0;
    for(const lm of state.landmarks) {
      if(!selected(lm.kind))continue;
      const sx=size/2+(lm.x-p.x)*ppm;
      const sy=size/2-(lm.z-p.z)*ppm;
      if(sx < -25||sx>size+25||sy< -15||sy>size+15)continue;
      const tint=palette[lm.kind]||"#f4cc7b";
      // Larger, high-contrast category-colored pins; the camera arrow
      // remains gold and is never confused with a place category.
      ctx.fillStyle=tint;
      ctx.strokeStyle="#061923";
      ctx.lineWidth=2.1;
      ctx.beginPath();
      ctx.arc(sx,sy,4.5,0,2*Math.PI);
      ctx.stroke();ctx.fill();
      drawn++;
      // Text appears only at closer zoom to reduce label crowding.
      if(state.width<=750) {
        ctx.lineWidth=3;
        ctx.strokeText(lm.short||lm.name,sx+6,sy-4);
        ctx.fillStyle=tint;
        ctx.fillText(lm.short||lm.name,sx+6,sy-4);
      }
    }
    ctx.restore();
    canvas.dataset.paintedLandmarkCount=String(drawn);
  }
  function updateWorldLabels(p) {
    if(!layer||!state.landmarks.length)return;
    // Labels are always horizontal HTML anchored to 3D points. They must
    // NOT disappear merely because the user turns or moves. Only hide
    // labels genuinely outside the camera frustum, behind us or past the
    // far visibility limit.
    const rect=document.querySelector("canvas:not(#minimap-canvas):not(#webgpu-control)")
      ?.getBoundingClientRect();
    const left=rect?.left??0,top=rect?.top??0;
    const w=rect?.width||window.innerWidth,h=rect?.height||window.innerHeight;
    const aspect=w/Math.max(1,h);
    const cp=Math.cos(p.pitch),sp=Math.sin(p.pitch);
    const sy=Math.sin(p.yaw),cy=Math.cos(p.yaw);
    const fx=sy*cp,fy=sp,fz=cy*cp;
    // Geographic X east, Z north, Y up is left-handed. The 3D viewer now
    // uses look_to_lh; use exactly its screen-right and screen-up axes.
    const rx=cy,rz=-sy;
    const ux=-sy*sp,uy=cp,uz=-cy*sp;
    const tan=Math.tan(68*Math.PI/360);
    let visible=0;
    for(const mark of state.landmarks) {
      const el=mark.el;if(!el)continue;
      if(!selected(mark.kind)){el.hidden=true;continue;}
      const dx=mark.x-p.x,dy=mark.y-p.y,dz=mark.z-p.z;
      const distance=Math.hypot(dx,dy,dz);
      const depth=dx*fx+dy*fy+dz*fz;
      if(depth<=0.1||distance>1800) {el.hidden=true;continue;}
      const hor=(dx*rx+dz*rz)/(depth*tan*aspect);
      const ver=(dx*ux+dy*uy+dz*uz)/(depth*tan);
      if(!Number.isFinite(hor)||!Number.isFinite(ver)||
          Math.abs(hor)>1.02||Math.abs(ver)>1.02) {
        el.hidden=true;continue;
      }
      el.hidden=false;
      // The DOM element itself never receives a rotate transform.
      el.style.left=(left+(hor+1)*w/2).toFixed(1)+"px";
      el.style.top=(top+(1-ver)*h/2).toFixed(1)+"px";
      el.style.opacity="1";
      visible++;
    }
    layer.setAttribute("data-labels-visible",String(visible));
    layer.setAttribute("data-labels-mode","view-frustum");
  }

  function draw(p) {
    if(!state.city)return;
    const ppm=size/state.width;
    const dx=(p.x-state.cx)*ppm,dz=(p.z-state.cz)*ppm;
    if(!state.cache||Math.abs(dx)>245||Math.abs(dz)>245)rebuild(p.x,p.z);
    const cropX=(bufferSize-size)/2+(p.x-state.cx)*ppm;
    const cropY=(bufferSize-size)/2-(p.z-state.cz)*ppm;
    ctx.drawImage(buffer,cropX*dpr,cropY*dpr,size*dpr,size*dpr,
      0,0,size,size);
    drawLandmarks(p,ppm);
    ctx.save();
    ctx.translate(size/2,size/2);
    // Positive yaw means clockwise from north (+Z) toward east (+X);
    // Canvas positive rotation is clockwise too. No yaw negation.
    ctx.rotate(p.yaw);
    ctx.fillStyle="#ffe39a";
    ctx.strokeStyle="#0e2630";
    ctx.lineWidth=2;
    ctx.beginPath();
    ctx.moveTo(0,-12);
    ctx.lineTo(-7,9);
    ctx.lineTo(0,5);
    ctx.lineTo(7,9);
    ctx.closePath();
    ctx.stroke();ctx.fill();
    ctx.restore();
    updateWorldLabels(p);
    state.frame++;
    canvas.setAttribute("data-map-ready","true");
    canvas.setAttribute("data-map-frame",String(state.frame));
    canvas.setAttribute("data-map-redraws",String(state.mapRedraws));
    canvas.setAttribute("data-map-width-m",String(state.width));
    canvas.setAttribute("data-player-x",String(p.x));
    canvas.setAttribute("data-player-z",String(p.z));
    canvas.setAttribute("data-heading-east",Math.sin(p.yaw).toFixed(6));
    canvas.setAttribute("data-heading-north",Math.cos(p.yaw).toFixed(6));
    if(state.frame%8===0){
      scaleLabel.textContent=Math.round(state.width)+" m across";
      const origin=state.city.region.origin;
      const lon=origin[0]+p.x/(radius*Math.cos(origin[1]*Math.PI/180));
      const lat=origin[1]+p.z/radius;
      coordsLabel.textContent=lat.toFixed(5)+"° N, "+
        Math.abs(lon).toFixed(5)+"° W";
    }
  }
  function loop(){
    if(state.city&&!document.hidden) {
      const p=pose();
      state.lastPose=p;
      draw(p);
    }
    requestAnimationFrame(loop);
  }
  requestAnimationFrame(loop);
  // The Rust viewer downloads and parses the city exactly once. A browser
  // event passes the same validated payload into the map, avoiding the extra
  // network request that previously failed on intermittent connections.
  let landmarksStarted=false;
  let citySource="";
  const buildingSearch=document.getElementById("named-building-search");
  const buildingResults=document.getElementById("named-building-results");
  const namedCount=document.getElementById("named-building-count");
  let namedBuildings=[];
  function renderNamedBuildings(){
    if(!buildingResults)return;
    const filter=(buildingSearch?.value||"").trim().toLocaleLowerCase();
    const matches=namedBuildings.filter(b=>b.name.toLocaleLowerCase().includes(filter));
    const limited=matches.slice(0,60);
    buildingResults.replaceChildren();
    if(!limited.length){
      const item=document.createElement("p");
      item.textContent=filter?"No source-named buildings match that search.":
        "No building names supplied by the source for this area.";
      buildingResults.append(item);
      return;
    }
    for(const b of limited){
      const item=document.createElement("div");
      item.className="named-building-item";
      item.setAttribute("role","listitem");
      const title=document.createElement("strong");
      title.textContent=b.name;
      const detail=document.createElement("small");
      const facts=[];
      if(Number.isFinite(b.height_m)&&b.height_m>0)
        facts.push("recorded height: "+b.height_m.toFixed(1)+" m");
      if(Number.isFinite(b.levels)&&b.levels>0)
        facts.push("source floors: "+b.levels);
      facts.push("footprint: "+b.source_id);
      detail.textContent=facts.join(" · ");
      item.append(title,detail);
      buildingResults.append(item);
    }
    if(matches.length>limited.length){
      const note=document.createElement("small");
      note.textContent="Showing "+limited.length+" of "+matches.length+"; refine search.";
      buildingResults.append(note);
    }
  }
  buildingSearch?.addEventListener("input",renderNamedBuildings);
  function fillNamedBuildings(city){
    namedBuildings=city.buildings
      .filter(b=>typeof b.name==="string" && b.name.trim())
      .sort((a,b)=>a.name.localeCompare(b.name));
    if(namedCount)namedCount.textContent=String(namedBuildings.length);
    const panel=document.getElementById("named-buildings-panel");
    if(panel)panel.hidden=namedBuildings.length===0;
    renderNamedBuildings();
  }

  function applyCity(city,source) {
    if(state.city) return;
    if(!city?.region||!Array.isArray(city.streets)||
       !Array.isArray(city.buildings))
      throw new Error("Invalid geographic city scene");
    state.city=city;
    fillNamedBuildings(city);
    citySource=source;
    state.cache=false;
    canvas.setAttribute("data-map-source",source);
    scaleLabel.textContent="Loading map...";
    coordsLabel.textContent="Geography loaded";
    // Render an initial map even before the next animation frame.
    draw(pose());
    if(!landmarksStarted) {
      landmarksStarted=true;
      void loadOptionalLandmarks();
    }
  }
  function useSharedCity() {
    if(state.city)return true;
    const payload=window.__urbariumCityJson;
    if(typeof payload!=="string")return false;
    try {
      applyCity(JSON.parse(payload),"rust-shared");
      delete window.__urbariumCityJson;
      return true;
    }catch(error){
      console.error("Urbarium shared city parse failed",error);
      scaleLabel.textContent="Map data error";
      coordsLabel.textContent=String(error);
      return false;
    }
  }
  function useSharedTerrain() {
    const payload=window.__urbariumTerrainJson;
    if(typeof payload!=="string")return false;
    try {
      state.terrain=JSON.parse(payload);
      delete window.__urbariumTerrainJson;
      for(const mark of state.landmarks)mark.y=landmarkHeight(mark);
      return true;
    }catch(error){
      console.warn("Urbarium shared elevation unavailable",error);
      return false;
    }
  }
  async function loadOptionalLandmarks() {
    // Landmarks are an enhancement. Their failure must never black out the
    // basic street/building map. Retry them independently a few times.
    for(const delay of [0,1000,3500]) {
      if(delay)await new Promise(resolve=>setTimeout(resolve,delay));
      try {
        const response=await fetch("./assets/landmarks.json");
        if(!response.ok)throw new Error("Landmarks HTTP "+response.status);
        const doc=await response.json();
        installLandmarks(doc);
        return;
      }catch(error){
        console.warn("Urbarium landmark labels delayed",error);
      }
    }
  }
  async function retryMapLoad() {
    if(useSharedCity()||state.city) {
      state.cache=false;
      return;
    }
    scaleLabel.textContent="Retrying map...";
    coordsLabel.textContent="Waiting for the same geographic dataset";
    try {
      // Last-resort explicit retry only. Normal startup NEVER makes a
      // second request; Rust owns the initial city fetch.
      const response=await fetch("./assets/downtown-la.json",{cache:"no-cache"});
      if(!response.ok)throw new Error("City HTTP "+response.status);
      applyCity(await response.json(),"manual-retry");
    }catch(error){
      scaleLabel.textContent="Map unavailable · Retry";
      coordsLabel.textContent="Network error; the 3D viewer can keep running";
      console.warn("Urbarium map retry failed",error);
    }
  }
  window.addEventListener("urbarium-city-ready",useSharedCity);
  window.addEventListener("urbarium-terrain-ready",useSharedTerrain);
  document.getElementById("map-retry")?.addEventListener("click",retryMapLoad);
  scaleLabel.textContent="Waiting for city data...";
  coordsLabel.textContent="Shares the Rust viewer's data";
  useSharedCity();
  useSharedTerrain();
})();
