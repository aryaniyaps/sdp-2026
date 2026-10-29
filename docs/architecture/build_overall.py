"""Build the editable, source-grounded Memory Engine overview (standard draw.io shapes)."""
from pathlib import Path
import xml.etree.ElementTree as E
from html import escape

HERE=Path(__file__).resolve().parent
W,H=2000,1740
mx=E.Element('mxfile',host='app.diagrams.net',version='32.4.1')
diagram=E.SubElement(mx,'diagram',id='memory-engine-overall',name='Overall architecture')
model=E.SubElement(diagram,'mxGraphModel',dx=str(W),dy=str(H),grid='1',gridSize='10',guides='1',tooltips='1',connect='1',arrows='1',fold='1',page='1',pageScale='1',pageWidth=str(W),pageHeight=str(H),background='#FFFFFF',math='0',shadow='0')
root=E.SubElement(model,'root'); E.SubElement(root,'mxCell',id='0'); E.SubElement(root,'mxCell',id='1',parent='0')
navy='#182D45'; muted='#516379'; blue='#2463A6'; purple='#7953AC'; teal='#167F83'; green='#26754B'; amber='#A16B17'
boxes={}

def box(id,x,y,w,h,label='',fill='#FFFFFF',stroke='#CAD4DF',size=18,extra='',parent='1'):
    style=f'rounded=1;arcSize=12;absoluteArcSize=1;whiteSpace=wrap;html=1;fillColor={fill};strokeColor={stroke};strokeWidth=1.5;fontFamily=Helvetica;fontColor={navy};fontSize={size};align=left;verticalAlign=top;spacing=16;{extra}'
    c=E.SubElement(root,'mxCell',id=id,value=label,style=style,vertex='1',parent=parent)
    E.SubElement(c,'mxGeometry',x=str(x),y=str(y),width=str(w),height=str(h),**{'as':'geometry'})
    boxes[id]=(x,y,w,h)
    return id

def text(id,x,y,w,h,label,size=18,color=navy,bold=False,align='left'):
    return box(id,x,y,w,h,label,stroke='none',fill='none',size=size,extra=f'spacing=0;fontColor={color};fontStyle={1 if bold else 0};align={align};')

def edge(id,src,dst,label='',color=muted,dash=False,ex=1,ey=.5,ix=0,iy=.5,points=(),both=False):
    style=f'edgeStyle=orthogonalEdgeStyle;rounded=0;html=1;endArrow=block;endFill=1;strokeWidth=2;strokeColor={color};fontColor={color};fontFamily=Helvetica;fontSize=15;labelBackgroundColor=#FFFFFF;exitX={ex};exitY={ey};exitDx=0;exitDy=0;entryX={ix};entryY={iy};entryDx=0;entryDy=0;jumpStyle=arc;jumpSize=8;'
    if dash: style+='dashed=1;dashPattern=6 4;'
    if both: style+='startArrow=block;startFill=1;'
    c=E.SubElement(root,'mxCell',id=id,value=label,style=style,edge='1',parent='1',source=src,target=dst)
    g=E.SubElement(c,'mxGeometry',relative='1',**{'as':'geometry'})
    if points:
        a=E.SubElement(g,'Array',**{'as':'points'})
        for x,y in points: E.SubElement(a,'mxPoint',x=str(x),y=str(y))

def lines(*ss): return '<br>'.join(ss)
def title(s): return f'<b>{s}</b>'

# Title and reading key.
text('eyebrow',40,25,1400,25,'SDP 2026  /  MEMORY ENGINE  /  ARCHITECTURE 01',16,blue,True)
text('title',40, 60,1850, 50,'From conversation to evidence-backed memory',36,navy,True)
text('subtitle',40,115,1850,34,'Four runtime modules + one offline model pipeline. Read the system above, then follow the same example below.',20,muted)

box('engine',285,205,1670,420,fill='#F7F9FC',stroke='#AEBCCC',extra='dashed=1;dashPattern=6 4;')
text('engine-title',310,218,1550,32,'MEMORY ENGINE  |  One Rust / Axum service: API handlers + asynchronous workers',19,navy,True)
# Client entry, document symbol.
box('client',40,285,210,235,lines(title('User / conversation'),'', 'Pi extension', 'or React dashboard','', 'Example: E2', '“Maya moved to', 'Bengaluru today.”'),fill='#F1F5FA',size=19,extra='shape=document;size=0.08;spacingBottom=25;')
text('client-note',40,540,215,92,'Pi / dashboard entry<br>Namespace: guide-demo<br>E2: 10 Oct 2026',17,muted)

modules=[
('m1',315,blue,'M1  Evidence Acquisition<br>&amp; Processing',['Validate + deduplicate requests','Persist original events + chunks','Durable jobs, leases and retries'],'E2 → stored event + extraction job'),
('m2',725,purple,'M2  Knowledge Extraction<br>&amp; Consolidation',['Window evidence + existing facts','Extract and validate quoted claims','Review corrections; derive observations'],'Maya · lives_in · Bengaluru · E2'),
('m3',1135,teal,'M3  Temporal Knowledge<br>&amp; Graph Management',['Resolve entities; apply fact updates','Preserve validity, conflicts + sources','Invalidate stale support; project graph'],'A2 active; A1 preserved in history'),
('m4',1545,green,'M4  Hybrid Retrieval<br>&amp; Grounded Reasoning',['Plan time scope; text + vector search','Expand graph; recheck facts; fuse ranks','Pack cited context; optionally reflect'],'Maya lives in Bengaluru. [A2]')]
for id,x,c,heading,body,out in modules:
    box(id,x,280,370,280,fill='#FFFFFF',stroke=c)
    box(id+'-badge',x+15,295,8,62,fill=c,stroke='none',extra='spacing=0;')
    text(id+'-title',x+35,298,320,62,heading,22,c,True)
    text(id+'-body',x+20,380,330,100,lines(*body),18)
    box(id+'-out',x+15,500,340,43,out,fill={'m1':'#EDF4FE','m2':'#F4EFFA','m3':'#EAF7F5','m4':'#EEF7EF'}[id],stroke='none',size=16,extra='spacing=10;fontStyle=1;')
edge('retain', 'client','m1','retain',blue)
edge('queue','m1','m2','jobs',blue,True)
edge('claims','m2','m3','apply',purple)
edge('read','m4','m3','read',green,ex=0,ix=1,both=True)
edge('query','client','m4','Where does Maya live now?  →  recall / reflect',green,ex=.5,ey=0,ix=.5,iy=0,points=[(145,175),(1730,175)])
edge('feedback','m3','m2','Existing facts / active supports for extraction and consolidation',teal,ex=.25,ey=1,ix=.75,iy=1,points=[(1227.5,595),(1002.5,595)])

# Dependencies are drawn once, with consumers explicitly named.
box('pg',340,710,350,140,lines(title('PostgreSQL + pgvector'),'Authoritative evidence, facts,','vectors, jobs, history and traces'),fill='#EDF4FE',stroke=blue,size=20,extra='shape=cylinder3;boundedLbl=1;backgroundOutline=1;size=16;verticalAlign=middle;align=center;spacingTop=20;')
text('pg-users',345,858,350,28,'Shared by M1, M2, M3 and M4',16,muted,align='center')
edge('persist','m1','pg','persist evidence + jobs',blue,ex=.5,ey=1,ix=.5,iy=0)
edge('knowledge-write','m3','pg','transactional knowledge updates',teal,ex=.1,ey=1,ix=.9,iy=0,points=[(1172,655),(655,655)])
box('ollama',750,710,340,140,lines(title('Ollama model serving'),'Generation: local memory worker','Embeddings: Qwen3 embedding'),fill='#F4EFFA',stroke=purple,size=20,extra='verticalAlign=middle;align=center;')
text('model-users',750,858,340,28,'Inference used by M2 and M4',16,muted,align='center')
edge('inference','m2','ollama','model calls',purple,ex=.5,ey=1,ix=.5,iy=0,both=True)
box('neo',1150,710,330,140,lines(title('Neo4j'),'Derived graph for traversal','Rebuildable from PostgreSQL'),fill='#EAF7F5',stroke=teal,size=20,extra='shape=cylinder3;boundedLbl=1;backgroundOutline=1;size=16;verticalAlign=middle;align=center;spacingTop=20;')
text('neo-users',1150,858,330,28,'M4 validates graph hits in PostgreSQL',16,muted,align='center')
edge('project','m3','neo','projection jobs',teal,True,ex=.6,ey=1,ix=.5,iy=0)
box('result',1545,710,370,140,lines(title('Returned to the caller'),'<b>Maya lives in Bengaluru. [A2]</b>','[A2] links to E2’s original quote.','Or: return cited context to Pi.'),fill='#EEF7EF',stroke=green,size=19,extra='shape=document;size=0.1;')
edge('return','m4','result','response',green,ex=.5,ey=1,ix=.5,iy=0)
text('boundary-note',40,1167,1910,26,'Retain acknowledges durable evidence before extraction finishes. Shared stores and models are shown once; modules are logical boundaries.',17,muted)

# Offline lane. Only released weights cross into serving.
box('training',40,955,1915,200,fill='#FFFBF2',stroke=amber,extra='dashed=1;dashPattern=6 4;')
text('m5-title',65,973,1750,33,'M5  Model Training, Evaluation & Deployment  |  Offline pipeline',24,amber,True)
train=[('data',70,'Curated conversations<br>+ teacher labels'),('tune',445,'Fine-tune the<br>local worker model'),('eval',820,'Held-out model<br>evaluation'),('export',1195,'Merge adapter<br>+ export GGUF'),('release',1570,'Install in Ollama<br>+ live engine checks')]
for id,x,s in train: box(id,x,1030,330,90,s,fill='#FFFFFF',stroke='#C9AB74',size=20,extra='align=center;verticalAlign=middle;')
for (a,*_),(b,*__) in zip(train,train[1:]): edge('train-'+a,a,b,color=amber)
edge('deployment','release','ollama','Released worker weights (embedding model is separate)',amber,True,ex=.5,ey=0,ix=.5,iy=1,points=[(1735,920),(920,920)])
text('training-note',65,1127,1800,24,'Training produces the worker model. A live user message does not pass through training; no automatic retraining path is shown.',16,amber)

# Walkthrough: transformations in the same named modules.
text('walk-title',40,1200,1890,40,'One example, five visible representations',29,navy,True)
text('walk-subtitle',40,1248,1900,32,'Illustrative expected behavior, not a live inference receipt. Two dated episodes in one namespace; process the first before the second.',18,muted)
walk=[
('w0',40,muted,'INPUT  /  Original conversation',[
'<b>E1 · 01 Oct 2026</b>','“Maya lives in Chennai.”','',
'<b>E2 · 10 Oct 2026</b>','“Maya moved to Bengaluru today.”','',
'Later question:','<b>Where does Maya live now?</b>']),
('w1',435,blue,'M1  /  Durable evidence',[
'<b>What changes?</b>','Text becomes attributed records.','',
'namespace: guide-demo','episode: E1 / E2','role: user; timestamp retained','original quotes stored as chunks','',
'<b>Result:</b> evidence saved + jobs queued']),
('w2',830,purple,'M2  /  Validated claims',[
'<b>What changes?</b>','Text becomes structured claims.','',
'Maya · lives_in · Chennai [E1]','Maya · lives_in · Bengaluru [E2]','E2: explicit correction, dated 10 Oct','Each claim retains its exact quote.','',
'Consolidation requires 2+ active supports;','this example needs no observation.']),
('w3',1225,teal,'M3  /  Knowledge with history',[
'<b>What changes?</b>','The correction updates a fact’s validity.','',
'<b>A1: Chennai — superseded</b>','valid: 01 Oct ≤ time &lt; 10 Oct','<b>A2: Bengaluru — active</b>','valid: 10 Oct onward','A2 supersedes A1; sources preserved.','',
'Neo4j receives a derived projection.']),
('w4',1620,green,'M4  /  Cited final answer',[
'<b>What changes?</b>','A question becomes selected evidence.','',
'Current-time filter selects A2.','Ranked evidence is packed for recall.','Reflection uses that evidence:','',
'<b>“Maya lives in Bengaluru.” [A2]</b>','Citation → E2’s original source quote.','Past-date queries can still find A1.'])]
for id,x,c,heading,body in walk:
    box(id,x,1310,340,310,fill='#FFFFFF',stroke=c)
    box(id+'-head',x,1310,340,48,heading,fill=c,stroke=c,size=17,extra='fontColor=#FFFFFF;fontStyle=1;spacing=13;')
    text(id+'-txt',x+16,1373,308,235,lines(*body),17)
for (a,*_),(b,*__) in zip(walk,walk[1:]): edge('walk-'+a,a,b,color=muted)
text('conditions',40,1640,1900, 30,'Example assumes the same entity/predicate, a supported correction with confidence ≥ 0.8, and completed jobs. E1/E2/A1/A2 are teaching labels; API IDs are UUIDs.',16,muted)
text('legend',40,1685,1900,27,'SYMBOLS   Rounded box: processing / model service     Document: input or output     Cylinder: database     Solid arrow: request or data access     Dashed arrow: background job / model delivery',16,muted)
text('footer',40,1720,1900,18,'Source: current working tree based on 177878c  •  13 Oct 2026  •  Includes local claim-review changes; source-checked, not a deployment verification',13,muted)

E.indent(mx,space='  ')
E.ElementTree(mx).write(HERE/'overall-architecture.drawio',encoding='utf-8',xml_declaration=True)
print(HERE/'overall-architecture.drawio')
