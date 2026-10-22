import { it, vi, expect } from "vitest";
import { ref } from "vue";
import { performance } from "node:perf_hooks";
import { writeFileSync } from "node:fs";
import { createMappingActions } from "../../src/stores/merge/mapping";
import { createPreviewActions, createPreviewState } from "../../src/stores/merge/preview";
import { defaultOptions, type SourceTable } from "../../src/types";
const mocks=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock("../../src/native",()=>({invoke:mocks.invoke}));
vi.mock("element-plus",()=>({ElMessage:{error:vi.fn()}}));
it("measures metadata grouping and cached preview only",async()=>{
 const results:unknown[]=[];
 for(const [count,cols] of [[10,16],[10,64],[100,64],[1000,64],[10,512],[100,256]]){
  for(const distinct of [false,true]){
   for(let repeat=0;repeat<3;repeat++){
    const raw:SourceTable[]=Array.from({length:count},(_,i)=>{
     const headers=Array.from({length:cols},(_,c)=>`${distinct?`来源${i}_`:""}字段${c}_中文业务`);
     return {path:`C:\\测试数据\\input${i}.xlsx`,sheet_name:"业务数据",kind:"Workbook",header_row:1,header_rows:1,suggested_header_row:1,estimated_rows:1_000_000,enabled:true,headers,mappings:headers.map((name,c)=>({source_index:c,source_name:name,target_name:`字段${c}_中文业务`,enabled:true,transform:"None"}))};
    });
    const sources=ref(raw);const options=ref(defaultOptions());const planHeaders=ref(raw[0].mappings.map(m=>m.target_name));
    const actions=createMappingActions({sources,options,planHeaders,invalidateResults:()=>{}});
    let at=performance.now();const groups=actions.fieldGroups.value;const groupMs=performance.now()-at;
    expect(groups.length).toBe(distinct?count*cols:cols);
    at=performance.now();const encoded=JSON.stringify({tables:sources.value,options:options.value});const jsonMs=performance.now()-at;
    at=performance.now();sources.value[0].mappings[0].target_name="独立字段";const next=actions.fieldGroups.value;const editMs=performance.now()-at;
    expect(next.length).toBe(groups.length+(distinct||count===1?0:1));
    results.push({count,cols,distinct,repeat,groupMs,jsonMs,editMs,jsonBytes:Buffer.byteLength(encoded)});
   }
  }
 }
 const sources=ref<SourceTable[]>([{path:"wide.xlsx",sheet_name:"数据",kind:"Workbook",header_row:1,header_rows:1,suggested_header_row:1,estimated_rows:1_000_000,enabled:true,headers:Array.from({length:512},(_,i)=>`字段${i}`),mappings:[]}]);
 const state=createPreviewState();const actions=createPreviewActions({...state,sources,options:ref(defaultOptions())});
 mocks.invoke.mockResolvedValue({headers:sources.value[0].headers,rows:Array.from({length:30},()=>Array.from({length:512},()=>"复杂中文000001234567890123456789"))});
 await actions.showSourcePreview(0);expect(mocks.invoke).toHaveBeenCalledTimes(1);
 const cache=[];for(let i=0;i<3;i++){actions.closeSourcePreview();const at=performance.now();await actions.showSourcePreview(0);cache.push(performance.now()-at);}
 expect(mocks.invoke).toHaveBeenCalledTimes(1);
 const out={results,cache512ColsMs:cache,note:"Node/Vue computed and JSON only; native IPC transport and DOM excluded"};
 writeFileSync(process.env.SF_FRONTEND_RESULTS!,JSON.stringify(out,null,2));console.log("FRONTEND_BENCH_COMPLETE",results.length);
},120000);
