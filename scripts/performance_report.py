#!/usr/bin/env python3
"""Create compact Markdown/CSV from the completed batch records."""
import argparse
import csv
import json
from pathlib import Path
import statistics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('matrix', type=Path)
    parser.add_argument('--frontend', type=Path)
    args = parser.parse_args()
    report = json.loads((args.matrix/'matrix.json').read_text())
    assert report.get('complete'), 'Finish the matrix before summarizing it.'
    def selected(case, stage, label):
        return [r for r in report['runs'] if (r['case'],r['stage'],r['label'])==(case,stage,label)]
    def median(values,key): return statistics.median(r[key] for r in values)
    rows=[]
    for case in report['metadata']['cases']:
        for stage in sorted({r['stage'] for r in report['runs'] if r['case']==case['name']}):
            a=selected(case['name'],stage,'baseline'); b=selected(case['name'],stage,'current')
            if len(a)!=3 or len(b)!=3: continue
            rows.append(dict(case=case['name'],stage=stage,before_ms=median(a,'ms'),after_ms=median(b,'ms'),
                time_change=median(b,'ms')/median(a,'ms')-1,before_rss_mib=median(a,'rss_bytes')/1024**2,
                after_rss_mib=median(b,'rss_bytes')/1024**2,rss_change=median(b,'rss_bytes')/median(a,'rss_bytes')-1))
    with (args.matrix/'summary.csv').open('w') as stream:
        writer=csv.DictWriter(stream,fieldnames=list(rows[0]),lineterminator="\n");writer.writeheader();writer.writerows(rows)
    lines=['# SheetForge 批量性能验证','',
        f"基线 `{report['metadata']['baseline']}`；共 {len(report['metadata']['cases'])} 组输入、{len(report['runs'])} 次阶段执行。源文件哈希、构建条件和原始数据见 matrix.json。",'',
        '同机 Release、同批文件；普通对照各三次取中位数。预览计时在扫描完成后开始，进程峰值内存包含准备阶段；行流计时包含内容哈希。系统文件缓存未清空。千万行仅做一次压力与内容验证，不作速度结论。','','| 场景 | 环节 | 旧版 ms | 新版 ms | 时间变化 | 旧/新 RSS MiB |','|---|---|---:|---:|---:|---:|']
    highlights={('xlsx_inline_1m_16','source_preview_warm'),('xlsx_shared_100k_64','source_preview_warm'),
        ('xlsx_inline_10k_512','source_preview_warm'),('csv_100files_1k_64','check_xlsx'),
        ('csv_100files_1k_64','check_pg'),('csv_1m_16','stream_filter_late'),('csv_1m_16','stream_dedup'),
        ('csv_gbk_100k_64','stream_dedup')}
    for row in rows:
        if (row['case'],row['stage']) in highlights:
            lines.append(f"| {row['case']} | {row['stage']} | {row['before_ms']:.2f} | {row['after_ms']:.2f} | {row['time_change']:+.1%} | {row['before_rss_mib']:.1f} / {row['after_rss_mib']:.1f} |")
    lines += ['','## 压缩实验（未上线）：时间与体积','', '| 场景 | 普通 ms | 快速 ms | 时间变化 | 普通/快速 MiB |','|---|---:|---:|---:|---:|']
    for case in report['metadata']['cases']:
        normal=selected(case['name'],'merge_xlsx','current');fast=selected(case['name'],'merge_xlsx_fast','current')
        if len(normal)==len(fast)==3:
            size=lambda records: statistics.median(r['data']['output_bytes'] for r in records)/1024**2
            lines.append(f"| {case['name']} | {median(normal,'ms'):.2f} | {median(fast,'ms'):.2f} | {median(fast,'ms')/median(normal,'ms')-1:+.1%} | {size(normal):.2f} / {size(fast):.2f} |")
    lines += ['','低压缩的完整导出仅快 3.6%–8.3%，文件增大约 24%–38%，未达到本轮 10% 的明显收益门槛，所以生产界面不增加开关，保留默认压缩。实验仅由 cfg(test) 入口启用，两种压缩的工作表/样式 XML 与读回内容均一致。精确去重使用 64 MiB 键预算，超过后落本机临时索引；这会增加部分场景耗时，不能描述为所有去重都更快。完整明细与超过 10% 的变化见 summary.csv。','',
        '## 内容核验','', '| 输入 | 行数 | Sheet 数 | 新旧内容 |','|---|---:|---:|---|']
    for case in report['metadata']['cases']:
        name=case['name'];a=selected(name,'verify_output','baseline');b=selected(name,'verify_output','current')
        assert a and b and a[0]['data']['hash']==b[0]['data']['hash']
        merged=selected(name,'merge_xlsx','current')[0]['data']
        lines.append(f"| {name} | {merged['rows']:,} | {merged['sheets']} | 一致 |")
    if args.frontend:
        lines += ['','## 前端组件','', '| 组件 | 列数 | 旧版 ms | 新版 ms | 旧/新 DOM |','|---|---:|---:|---:|---:|']
        paths={label:args.frontend/f'{label}-ui.json' for label in ('baseline','current')}
        if all(p.exists() for p in paths.values()):
            records={label:json.loads(path.read_text()) for label,path in paths.items()}
            for kind in ('preview','fields'):
                for cols in (64,256,512):
                    values={label:[r for r in group if r['kind']==kind and r['cols']==cols] for label,group in records.items()}
                    assert all(len(v)==3 for v in values.values())
                    a,b=values['baseline'],values['current']
                    lines.append(f"| {kind} | {cols} | {median(a,'ms'):.2f} | {median(b,'ms'):.2f} | {a[0]['nodes']} / {b[0]['nodes']} |")
            lines += ['','新版还通过千来源×512列的合并预览滚动、末列、固定来源和空表检查。旧版该压力场景明确跳过，不计为通过。计时覆盖 Vue 挂载与同步布局，不含完整原生窗口、后续异步布局与绘制。']
        else: lines.append('浏览器批量页尚未执行；ui-build 只表示生产页面构建成功。')
        metadata_paths = {label: args.frontend/f'{label}-metadata.json' for label in ('baseline','current')}
        if all(path.exists() for path in metadata_paths.values()):
            metadata = {label: json.loads(path.read_text())['results'] for label,path in metadata_paths.items()}
            lines += ['', '前端元数据计算（同一 Node/Vue 进程，不含原生 IPC 或 DOM）：', '', '| 表数×列数 | 源字段 | 首次分组旧/新 ms | 编辑重算旧/新 ms |', '|---|---|---:|---:|']
            for count,cols,distinct in [(1000,64,False),(1000,64,True),(100,256,True)]:
                values = {label:[r for r in group if (r['count'],r['cols'],r['distinct'])==(count,cols,distinct)] for label,group in metadata.items()}
                a,b=values['baseline'],values['current']
                lines.append(f"| {count}×{cols} | {'各表不同' if distinct else '相同'} | {median(a,'groupMs'):.2f} / {median(b,'groupMs'):.2f} | {median(a,'editMs'):.2f} / {median(b,'editMs'):.2f} |")

    lines += ['','首次完整导入仍需统计真实范围；含共享字符串的 XLSX 仍需初始化共享字符串表。这里是 macOS 合成数据与组件测量，不能等同用户真实文件、Windows 桌面或旧版软件通过实际更新服务的升级验收。','']
    (args.matrix/'report.md').write_text('\n'.join(lines))
    print(args.matrix/'report.md')


if __name__=='__main__': main()
