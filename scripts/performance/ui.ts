import { createApp, h, ref, nextTick } from 'vue';
import { createPinia } from 'pinia';
import ElementPlus from 'element-plus';
import zhCn from 'element-plus/es/locale/lang/zh-cn';
import 'element-plus/dist/index.css';
import '../../src/styles.css';
import '../../tokens.css';
import '../../src/hallmark.css';
import PreviewTableGrid from '../../src/components/PreviewTableGrid.vue';
import MergedPreviewGrid from '../../src/components/MergedPreviewGrid.vue';
import FieldBatchView from '../../src/components/FieldBatchView.vue';
import { useMergeStore } from '../../src/stores/merge';
import type { PreviewTable, MergedPreview, SourceTable } from '../../src/types';

createApp({ setup() {
  const store=useMergeStore();
  const active=ref(''); const preview=ref<PreviewTable|null>(null); const merged=ref<MergedPreview|null>(null);
  const status=ref('就绪'); const results=ref<unknown[]>([]); const running=ref(false);
  const contentWidth=ref(900);
  let virtual = false;
  const frame=()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
  function assert(condition: unknown, message: string) { if (!condition) throw new Error(message); }
  const content=()=>document.querySelector('#content')!;
  async function run(kind:string,cols:number) {
    active.value=''; await nextTick(); await frame();
    const headers=Array.from({length:cols},(_,i)=>`字段${i}_中文业务`);
    if(kind==='preview') preview.value={headers,rows:Array.from({length:30},(_,r)=>headers.map((_,c)=>`0001234567890123456789 中文 ${r}-${c}`))};
    else {
      store.sources=Array.from({length:10},(_,i):SourceTable=>({path:`C:\\测试\\input${i}.xlsx`,sheet_name:'业务',kind:'Workbook',enabled:true,header_row:1,header_rows:1,suggested_header_row:1,estimated_rows:1000000,headers,mappings:headers.map((name,index)=>({source_index:index,source_name:name,target_name:name,enabled:true,transform:'None'}))}));
      store.planHeaders=headers;
    }
    const at=performance.now(); active.value=kind; await nextTick(); content().getBoundingClientRect(); await nextTick(); content().getBoundingClientRect();
    const ms=performance.now()-at; await frame();
    const nodes=content().querySelectorAll('*').length;
    if(kind==='preview') {
      const grid=content().querySelector<HTMLElement>('.sf-virtual-preview');
      if(grid) {
        virtual = true;
        assert(nodes<1500,'宽表创建了过多 DOM');
        grid.scrollLeft=grid.scrollWidth; grid.scrollTop=grid.scrollHeight; await frame();
        assert(content().textContent?.includes(`29-${cols-1}`),'滚动后缺少最后一个单元格');
        const fixed=grid.querySelector('.sf-virtual-fixed-cell')!;
        assert(Math.abs(fixed.getBoundingClientRect().left-grid.getBoundingClientRect().left)<3,'固定行号未保持位置');
        contentWidth.value=500; await nextTick(); await frame();
        assert(grid.clientWidth<=502,'缩小窗口后网格未跟随');
        contentWidth.value=900; await nextTick(); await frame();
      }
    } else {
      const inputs=content().querySelectorAll('input[placeholder="目标字段"]');
      assert(inputs.length===Math.min(cols,100) || !content().querySelector('.sf-field-pagination'), '字段分页数量异常');
      const pager=content().querySelector<HTMLButtonElement>('.btn-next');
      if(pager && cols>100) {
        pager.click(); await nextTick(); await frame();
        assert(!content().textContent?.includes('字段0_中文业务'),'翻页没有切换数据');
        const previous=content().querySelector<HTMLButtonElement>('.btn-prev')!; previous.click(); await nextTick(); await frame();
        const search=content().querySelector<HTMLInputElement>('input[placeholder="搜索字段"]')!;
        search.value=`字段${cols-1}_中文业务`; search.dispatchEvent(new Event('input',{bubbles:true})); await nextTick(); await frame();
        assert(content().querySelectorAll('input[placeholder="目标字段"]').length===1,'搜索未覆盖其他页');
        store.mappingSearch=''; await nextTick(); await frame();
      }
      const input=content().querySelector<HTMLInputElement>('input[placeholder="目标字段"]')!;
      input.focus(); input.value='独立字段 测试'; input.dispatchEvent(new Event('input',{bubbles:true})); await nextTick();
      assert(document.activeElement===input,'编辑字段时丢失焦点');
      assert(store.sources[0].mappings[0].target_name===headers[0],'输入草稿提前提交');
      input.dispatchEvent(new Event('change',{bubbles:true})); input.blur(); await nextTick(); await frame();
      assert(store.sources.every(table=>table.mappings[0].target_name==='独立字段 测试'),'字段提交未覆盖所选组');
    }
    results.value.push({kind,cols,ms,nodes,visibility:document.visibilityState,checks:'pass'});
  }
  async function mergedCheck() {
    if (!virtual) { results.value.push({kind:'merged',checks:'skipped: baseline renders all cells'}); return; }
    active.value=''; await nextTick();
    const headers=Array.from({length:512},(_,i)=>`字段${i}`);
    merged.value={headers,groups:Array.from({length:1000},(_,i)=>({source_index:i,source_file:`来源${i}.xlsx`,source_sheet:'业务',rows:i===999?[]:Array.from({length:5},(_,r)=>headers.map((_,c)=>`${i}:${r}:${c}`))}))};
    active.value='merged'; await nextTick(); await frame();
    const grid=content().querySelector<HTMLElement>('.sf-virtual-preview');
    if(grid) {
      assert(content().querySelectorAll('*').length<1500,'合并预览未限制可见行列');
      grid.scrollLeft=grid.scrollWidth; grid.scrollTop=grid.scrollHeight; await frame();
      assert(content().textContent?.includes('来源999.xlsx'),'最后的空来源丢失');
      assert(content().textContent?.includes('空表'),'空表状态丢失');
      assert(content().textContent?.includes('998:4:511'),'合并预览最后一列丢失');
      assert(grid.querySelector('.sf-merged-preview-group-start'),'来源分组样式丢失');
      const fixed=grid.querySelector('.sf-virtual-fixed-cell')!;
      assert(Math.abs(fixed.getBoundingClientRect().left-grid.getBoundingClientRect().left)<3,'来源固定列移出窗口');
    }
    results.value.push({kind:'merged',cols:512,groups:1000,checks:'pass'});
  }
  async function all() {
    running.value=true; status.value='测试中'; results.value=[];
    try {
      for(const kind of ['preview','fields']) for(const cols of [64,256,512]) for(let repeat=0;repeat<3;repeat++) await run(kind,cols);
      await mergedCheck(); status.value='完成';
    } catch(error) { status.value=`失败：${String(error)}`; }
    running.value=false;
  }
  return()=>h('div',{style:'padding:16px;height:100%;overflow:auto'},[
    h('button',{onClick:all,disabled:running.value},'运行全部界面测试'),h('span',{id:'status',style:'margin-left:16px'},status.value),
    h('pre',{id:'results',style:'white-space:pre-wrap;font-size:12px'},JSON.stringify(results.value)),
    h('div',{id:'content',style:`width:${contentWidth.value}px;max-width:100%;height:700px;overflow:auto`},
      active.value==='preview'?h(PreviewTableGrid,{preview:preview.value,maxHeight:440}):active.value==='fields'?h(FieldBatchView):active.value==='merged'?h(MergedPreviewGrid,{preview:merged.value,maxHeight:440}):null)
  ]);
} }).use(createPinia()).use(ElementPlus,{locale:zhCn}).mount('#app');
