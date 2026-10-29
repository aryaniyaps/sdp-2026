"""Module 1 architecture. Does not modify the overall diagram."""
from pathlib import Path
import xml.etree.ElementTree as E
B=Path(__file__).resolve().parent
f=E.Element('mxfile',host='Electron');d=E.SubElement(f,'diagram',id='module1-evidence',name='Module 1 - Evidence Acquisition')
g=E.SubElement(d,'mxGraphModel',grid='1',gridSize='10',page='1',pageScale='1',pageWidth='2640',pageHeight='2050',background='#FFFFFF');r=E.SubElement(g,'root')
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
# System boundaries.
for id,x,w,label,fill,col in [('pi',40,580,'PI CODING AGENT','#F5F8FC',blue),('service',690,1180,'MEMORY SERVICE · RUST','#F8FAFC',blue),('pg',1940,660,'POSTGRESQL + PGVECTOR','#F2F9F8',teal)]:
 node(id,x,180,w,1050,'',color='#B9C8D7',fill=fill,extra='dashed=1;dashPattern=6 4;')
 txt(id+'-label',x+25,200,w-50,45,'<b>'+label+'</b>',24,col)
txt('title',40,35,2520,60,'<b>Module 1 · Evidence Acquisition &amp; Processing</b>',38,'#182D45')
txt('sub',40,108,2520,40,'Capture → durable delivery → atomic persistence → extraction handoff',25)
# Recognizable document, component and disk symbols.
node('messages',110,295,440,165,'<b>Conversation · 10 October</b><br>E3 user: “Bengaluru is now my home city.”<br>E4 assistant: acknowledgment','shape=document;size=0.12;',gray,'#FFFFFF',22,'spacingBottom=28;')
node('capture',160,570,340,135,'<b>Evidence collector</b><br>Branch + namespace cursor','shape=component;',blue)
node('spool',160,920,340,155,'<b>Local evidence spool</b><br>Durable JSON files','shape=cylinder;size=0.12;',blue,'#EDF4FE')
node('api',780,930,320,130,'<b>Axum Retain API</b><br>POST /api/v2/retain','shape=component;',blue)
node('writer',780,570,320,135,'<b>SQLx evidence writer</b><br>Validate · SHA-256 dedupe','shape=component;',blue)
txt('guard',705,735,320,115,'1–1,000 events · role / text checks<br>Same key + same hash → reuse IDs<br>Changed payload → 409',20)
node('embed',1370,300,440,180,'<b>qwen3-embedding-cpu</b><br>Qwen3-Embedding 0.6B · Q8_0<br>CPU · Ollama /api/embed','shape=component;',purple,'#F4EFFA')
node('worker',1410,930,360,130,'<b>Job claim + lease</b><br>Shared store logic','shape=component;',blue)
# Database and persistent message queue symbols.
node('evidence',2050,515,440,205,'<b>Evidence store</b><br>sessions · episodes<br>raw_events · chunks<br>episode_sources','shape=cylinder;size=0.12;',teal,'#FFFFFF',24)
node('queue',2050,910,440,170,'',color=teal,fill='#FFFFFF')
for i in range(3):node('q'+str(i),2080+i*58,932,43,36,'',color=teal,fill='#EAF7F5')
txt('queue-label',2080,985,390,70,'<b>memory_jobs</b><br>Persistent extraction queue',23,teal)
txt('atomic',2000,770,530,80,'<b>One transaction</b><br>Evidence + job commit together',23,teal)
# Dataflow, examples embedded next to connections.
edge('collect','messages','capture','role + original text + timestamp',gray,'exitX=0.5;exitY=1;entryX=0.5;entryY=0;')
edge('persist-local','capture','spool','<b>RetainBatch</b><br>namespace: user:maya · session: S2<br>external_id: branch:S2:L4<br>events: [E3, E4]',blue,'exitX=0.5;exitY=1;entryX=0.5;entryY=0;')
edge('deliver','spool','api','Saved JSON<br>retry unchanged')
edge('validate','api','writer','',blue,'exitX=0.8;exitY=0;entryX=0.8;entryY=1;')
edge('write','writer','evidence','<b>Original evidence preserved</b><br>P2 → session S2<br>C3 → E3 · C4 → E4<br>P2 → ordered chunks [C3, C4]',teal)
edge('enqueue','writer','queue','<b>J2 · pending</b><br>kind: extract<br>payload: {episode_id: P2}',teal,'exitX=1;exitY=0.85;entryX=0;entryY=0.15;',[(1200,685),(1200,855),(1980,855),(1980,935)])
edge('claim','queue','worker','<b>Claim J2</b><br>running + lease',blue,'exitX=0;exitY=0.7;entryX=1;entryY=0.7;')
# Acknowledgment is a return path, not another service.
edge('ack','api','spool','<b>HTTP 200</b> · {episode_id: P2, job_id: J2, created: true}<br>After commit → remove spool file',blue,'exitX=0.5;exitY=1;entryX=0.5;entryY=1;',[(940,1160),(330,1160)])
# Async embedding uses stored originals; vector write goes back to chunks.
edge('async','writer','embed','New P2 committed<br>schedule source embedding',purple,'exitX=0.5;exitY=0;entryX=0;entryY=0.5;',[(940,390)],True)
edge('text-read','evidence','embed','C3 / C4 original text',purple,'exitX=0.5;exitY=0;entryX=1;entryY=0.25;',[(2270,355)])
edge('vector-write','embed','evidence','Text → 1,024-d vector<br>chunks.embedding',purple,'exitX=1;exitY=0.8;entryX=0;entryY=0.12;',[(1880,432),(1880,540)])
txt('worker-policy',1160,1090,490,100,'SKIP LOCKED · namespace lease<br>15-min lease · renew every 30s<br>5 attempts · backoff ≤ 300s',20)
# Concurrent Tokio worker pool: each loop uses the shared claim function.
node('pool',690,1320,1910,590,'',color=purple,fill='#FAF8FC',extra='dashed=1;dashPattern=6 4;')
txt('pool-title',720,1340,1840,45,'<b>Concurrent Tokio workers · Module 2 entry</b>',26,purple)
txt('pool-config',720,1390,1840,45,'Local configuration: 2 workers · default 4 · range 1–8 · extract + consolidate',22)
for i,x in enumerate([1000,2100]):
 node('worker'+str(i),x,1490,320,125,'<b>Worker '+str(i+1)+'</b><br>'+['J2 · user:maya','J7 · namespace B','Available','Available'][i],'shape=component;',purple)
 edge('claim'+str(i),'worker','worker'+str(i),'',purple,'exitX=0.85;exitY=1;entryX=0.5;entryY=0;',[(1716,1280),(2570,1280),(2570,1460),(x+160,1460)])
node('model',1440,1730,480,120,'<b>mem-extractor:general-v1</b><br>Ollama /api/generate','shape=component;',purple,'#F4EFFA')
for i,x in enumerate([1000,2100]):
 edge('model'+str(i),'worker'+str(i),'model','',purple,'exitX=0.5;exitY=1;entryX='+str([0.25,0.75][i])+';entryY=0;',[(x+160,1670)])
txt('pool-input',740,1700,610,120,'<b>J2 → P2 → ordered evidence</b><br>[(C3, E3 user text), (C4, E4 assistant text)]<br>→ model request',21,purple)
txt('pool-order',2010,1710,520,140,'<b>One active job per namespace</b><br>Other namespaces can run concurrently.<br>Earlier failed extraction blocks its lane.',21)
txt('pool-foot',720,1930,1850,75,'Separate projection worker: project / rebuild / clear_graph. Source embedding: 2 concurrent tasks, independent of this pool.',21)
txt('legend',40,1320,1180,115,'<b>Symbols</b>  Component = executable unit · cylinder = storage<br>Queue = durable jobs · document = source messages<br>Dashed arrow = asynchronous scheduling',20)
txt('scope',40,1480,580,105,'E3/E4 are excerpted from Maya’s example; P2/C3/C4/J2 abbreviate generated IDs.',18)
txt('boundary',40,1630,560,130,'<b>Module 1 ends with durable evidence.</b><br>Assertions and entities are created downstream.',22)
ids={c.get('id') for c in r};assert len(ids)==len(r)
for c in r:
 for a in ['source','target']:assert not c.get(a) or c.get(a) in ids
E.indent(f,space='  ');E.ElementTree(f).write(B/'module-1-evidence-acquisition.drawio',encoding='unicode')
