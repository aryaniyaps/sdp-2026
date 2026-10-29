"""Module 1 architecture. Does not modify the overall diagram."""
from pathlib import Path
import xml.etree.ElementTree as E
B=Path(__file__).resolve().parent
f=E.Element('mxfile',host='Electron');d=E.SubElement(f,'diagram',id='module2-knowledge',name='Module 2 - Extraction and Consolidation')
g=E.SubElement(d,'mxGraphModel',grid='1',gridSize='10',page='1',pageScale='1',pageWidth='3200',pageHeight='1910',background='#FFFFFF');r=E.SubElement(g,'root')
E.SubElement(r,'mxCell',id='0');E.SubElement(r,'mxCell',id='1',parent='0')
blue='#2463A6';teal='#167F83';purple='#7953AC';gray='#516379'
def node(id,x,y,w,h,v,shape='',color=blue,fill='#FFFFFF',font=23,extra=''):
 c=E.SubElement(r,'mxCell',id=id,parent='1',vertex='1',value=v,style=f'whiteSpace=wrap;html=1;rounded=0;fillColor={fill};strokeColor={color};strokeWidth=2;fontFamily=Helvetica;fontColor=#182D45;fontSize={font};align=center;verticalAlign=middle;spacing=12;'+shape+extra)
 E.SubElement(c,'mxGeometry',x=str(x),y=str(y),width=str(w),height=str(h),attrib={'as':'geometry'})
def txt(id,x,y,w,h,v,font=21,color=gray):node(id,x,y,w,h,v,color='none',fill='none',font=font,extra=f'align=left;spacing=0;fontColor={color};')
def edge(id,a,b,v='',color=blue,ports='exitX=1;exitY=0.5;entryX=0;entryY=0.5;',points=(),dash=False):
 c=E.SubElement(r,'mxCell',id=id,parent='1',edge='1',source=a,target=b,value=v,style=f'edgeStyle=orthogonalEdgeStyle;rounded=0;html=1;endArrow=block;endFill=1;strokeWidth=2.5;strokeColor={color};fontColor={color};fontFamily=Helvetica;fontSize=20;labelBackgroundColor=#FFFFFF;'+ports+('dashed=1;' if dash else ''))
 gg=E.SubElement(c,'mxGeometry',relative='1',attrib={'as':'geometry'})
 if points:
  arr=E.SubElement(gg,'Array',attrib={'as':'points'})
  for x,y in points:E.SubElement(arr,'mxPoint',x=str(x),y=str(y))
txt('title',40,35,3110,60,'<b>Module 2 · Knowledge Extraction &amp; Consolidation</b>',38,'#182D45')
txt('sub',40,105,3100,45,'Evidence → validated claims · committed facts → supported observations',25)
# Shared infrastructure, referenced by all purple generation components.
node('runtime',40,185,3120,160,'',color='#CBBADC',fill='#FAF8FC',extra='dashed=1;')
node('llm',80,205,790,115,'<b>Cached generation · mem-extractor:general-v1</b><br>Ollama /api/generate · structured JSON · think:false · temperature:0','shape=component;',purple,font=23)
node('cache',1150,205,620,115,'<b>PostgreSQL model_cache</b><br>model + prompt version + prompt hash','shape=cylinder;size=0.12;',teal,font=22)
edge('cache-read','cache','llm','cached response',teal,'exitX=0;exitY=0.5;entryX=1;entryY=0.5;')
txt('runtime-limits',1900,213,1180,100,'<b>Shared by extraction, review and consolidation</b><br>Local limits: 12,288 context / 3,072 output tokens<br>Cache miss → generate and store response',23)
# Extraction path.
node('extract-zone',40,400,3120,660,'',color='#B9C8D7',fill='#F7FAFD',extra='dashed=1;dashPattern=6 4;')
txt('extract-label',65,415,3040,40,'<b>EXTRACTION · extract job {episode_id: P2} · namespace user:maya</b>',25,blue)
node('input',80,515,350,175,'<b>PostgreSQL evidence</b><br>P2 → [C3/E3, C4/E4]<br>Existing assertion snapshot<br>≤ 80 active / contested','shape=cylinder;size=0.12;',teal,font=22)
node('prepare',680,525,340,145,'<b>Prompt preparation</b><br>Compact · window · fit<br>Original evidence retained','shape=component;',blue,font=23)
node('extract',1280,525,340,145,'<b>Claim extractor</b><br>extract-v3.0<br>Cached JSON generation','shape=component;',purple,fill='#F4EFFA',font=23)
node('validate',1880,525,340,145,'<b>Claim validator</b><br>Schema · enums · sources<br>Exact quotes + relation IDs','shape=component;',blue,font=23)
node('review',2510,525,540,145,'<b>Correction + entity review</b><br>claim-review-v1 · JSON + revalidation<br>May veto correction; adds grounded names','shape=component;',purple,fill='#F4EFFA',font=23)
edge('load','input','prepare','<b>E3 user · 10 October</b><br>“Bengaluru is now<br>my home city.”',teal)
edge('prompt','prepare','extract','<b>Window</b><br>source_index: 0 → E3<br>Existing: Maya / Chennai',blue)
edge('claims','extract','validate','<b>ClaimInput</b><br>Maya → home city → Bengaluru<br>fact · single · correction:true',purple)
edge('valid-review','validate','review','<b>Verified source pair</b><br>index: 0 + exact E3 quote',blue)
edge('repair','validate','extract','invalid → error + repair prompt · at most 2 repairs',blue,'exitX=0.5;exitY=0;entryX=0.5;entryY=0;',[(2050,485),(1450,485)],True)
txt('window-note',660,720,430,100,'Long events: head + tail after 1,600 chars<br>Event window budget: ≈ 2,800 tokens',20)
node('assembly',2510,840,540,140,'<b>Claim assembly</b><br>Drop secret-bearing statements<br>Window indices → episode indices','shape=component;',blue,font=23)
edge('reviewed','review','assembly','<b>Reviewed correction</b><br>“Bengaluru is now my home city.”<br>Optional entity: Bengaluru (place)',purple,'exitX=0.5;exitY=1;entryX=0.5;entryY=0;')
node('embed-extract',1880,840,340,140,'<b>Claim embeddings</b><br>qwen3-embedding-cpu<br>1,024-d vectors','shape=component;',purple,fill='#F4EFFA',font=22)
edge('statements','assembly','embed-extract','“Maya lives in Bengaluru.”',blue,'exitX=0;exitY=0.5;entryX=1;entryY=0.5;')
node('apply-extract',1280,840,340,140,'<b>Module 3</b><br>apply_extraction<br>Entities + assertions','shape=component;',teal,fill='#EAF7F5',font=23)
edge('persist-claims','embed-extract','apply-extract','Claims + optional vectors<br>+ model identity',purple,'exitX=0;exitY=0.5;entryX=1;entryY=0.5;')
txt('source-repair',80,840,1020,130,'<b>Validation always uses full source events.</b><br>Review: one repair; ungrounded added entities are omitted.<br>Missing source embeddings are retried before claim embedding.',22)
# Consolidation is a later job reading committed assertions.
node('consolidate-zone',40,1120,3120,560,'',color='#B9C8D7',fill='#F3F9F8',extra='dashed=1;dashPattern=6 4;')
txt('consolidate-label',65,1140,3040,40,'<b>CONSOLIDATION · separate job after Module 3 commits · serialized per namespace</b>',25,teal)
node('facts',80,1260,350,170,'<b>Committed assertions</b><br>Changed subjects only<br>≤ 100 active non-observations<br>Fewer than 2 → no-op','shape=cylinder;size=0.12;',teal,font=22)
node('consolidate',950,1270,360,145,'<b>Observation generator</b><br>consolidate-v2.3<br>Cached JSON generation','shape=component;',purple,fill='#F4EFFA',font=23)
node('validate-obs',1850,1270,360,145,'<b>Support validator</b><br>≥ 2 distinct supplied IDs<br>Subject belongs to a support','shape=component;',blue,font=23)
node('embed-obs',2670,1270,400,145,'<b>Observation embeddings</b><br>qwen3-embedding-cpu<br>1,024-d vectors','shape=component;',purple,fill='#F4EFFA',font=22)
edge('work-facts','facts','consolidate','<b>Maya’s active work facts</b><br>A1: employer = NovaTech<br>A2: role = software engineer<br>A3: work mode = remote',teal)
edge('observation','consolidate','validate-obs','<b>ObservationInput</b><br>“Maya works remotely as a<br>software engineer at NovaTech.”<br>supports: [A1, A2, A3]',purple)
edge('validated-obs','validate-obs','embed-obs','Supported statement<br>+ subject + support IDs',blue)
edge('repair-obs','validate-obs','consolidate','invalid → error + repair prompt · at most 2 repairs',blue,'exitX=0.5;exitY=0;entryX=0.5;entryY=0;',[(2030,1225),(1130,1225)],True)
node('apply-obs',2670,1520,400,120,'<b>Module 3</b><br>apply_consolidation','shape=component;',teal,fill='#EAF7F5',font=23)
edge('persist-obs','embed-obs','apply-obs','Observation + optional vector',purple,'exitX=0.5;exitY=1;entryX=0.5;entryY=0;')
txt('commit-note',80,1510,2280,120,'<b>Observations are proposed here; Module 3 rechecks supports and persists them.</b><br>Extraction and consolidation share the configured 2-worker pool; one active inference job per namespace.',23)
txt('legend',40,1730,3120,60,'Purple components call the shared Ollama model or embedder. Dashed arrows: bounded repair. Cylinders: PostgreSQL records. Embedding failures may leave vectors absent.',20)
txt('example-note',40,1810,3120,55,'Maya examples are illustrative; generated IDs are abbreviated. This module proposes claims and observations; it does not decide or persist residence supersession.',20)
ids={x.get('id') for x in r};assert len(ids)==len(r)
for x in r:
 for a in ('source','target'):assert not x.get(a) or x.get(a) in ids
E.indent(f,space='  ');E.ElementTree(f).write(B/'module-2-knowledge-extraction.drawio',encoding='unicode')
