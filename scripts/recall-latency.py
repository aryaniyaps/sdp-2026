"""Recall latency of a running service: 40 questions three times each in sequence, then 10 requests a second for 30 seconds.
Usage: scripts/recall-latency.py [namespace]      BASE_URL defaults to http://127.0.0.1:8080
The questions are about this project, so use a namespace that was built from its documents and source.
The service caches query embeddings (256 questions), so restart it first to measure uncached recall.
"""
import json, os, sys, time, statistics, datetime, urllib.request, concurrent.futures as cf, random
BASE=os.environ.get("BASE_URL","http://127.0.0.1:8080")+"/api/v2/recall"
if len(sys.argv)>2 or (len(sys.argv)==2 and sys.argv[1].startswith("-")):
    sys.exit(__doc__ or "usage: scripts/recall-latency.py [namespace]")
NAMESPACE=sys.argv[1] if len(sys.argv)>1 else "user:dev"
# 40 real questions about the stored project (docs and source), plus short follow-ups
Q=["how does the recall path fuse results","what is the reciprocal rank fusion constant","how are corrections stored as new versions",
"which tables hold assertions and their sources","how does the job queue prevent two workers on one namespace","what does the Pi extension do before a prompt",
"how is the graph projection rebuilt","what is the embedding model and its dimension","how are exact quotes validated","what happens when two sources disagree",
"which port does the memory service listen on","how does retain store an episode","what is the relevance cutoff for automatic recall","how does the worker repair a bad model reply",
"what does the valid_to column mean","how many facts can the extraction prompt carry","where does the Neo4j projection get its revision","what is the namespace for a user",
"how does the spool keep text when the service is down","which index serves vector search","how is full text search indexed","what is a contested assertion",
"how does supersession work","what does max_distance remove","how does the graph arm expand seeds","what is stored for each event",
"which model runs extraction by default","how is a lease renewed","what is fencing in the projection","how does recall plan a date window",
"what are observations and why two supports","which API routes exist in V2","how do I rebuild the graph","what does the healthz endpoint report",
"how big can a retain batch be","what happens to raw text before extraction finishes","how is the context budget enforced","what is event time versus valid time",
"which tests cover the Pi extension","how does the extension avoid retaining its own worker output"]
def call(q):
    body={"namespace":NAMESPACE,"query":q,"max_tokens":2048,"top_k":10,"max_distance":0.45,"include_raw":True,
          "reference_date":datetime.datetime.now(datetime.timezone.utc).isoformat()}
    t=time.perf_counter()
    r=urllib.request.Request(BASE,data=json.dumps(body).encode(),headers={"content-type":"application/json"})
    out=json.load(urllib.request.urlopen(r,timeout=60))
    return (time.perf_counter()-t)*1000, len(out["context"]), len(out["results"])
def pct(v,p): v=sorted(v); return v[min(len(v)-1,int(round(p/100*(len(v)-1))))]
for q in Q[:5]: call(q)  # warm up
seq=[call(q) for q in Q for _ in range(3)]
l=[s[0] for s in seq]
print("sequential n=%d p50=%.0f p95=%.0f max=%.0f ms"%(len(l),pct(l,50),pct(l,95),max(l)))
first=l[0::3]  # the first ask of each question; the other two find its embedding in the service's cache
print("first ask of each question n=%d p50=%.0f p95=%.0f max=%.0f ms  (uncached only if the service was started after the last run)"%(len(first),pct(first,50),pct(first,95),max(first)))
ctx=[s[1] for s in seq]; print("context chars median=%d p95=%d  tokens(chars/4) median=%d p95=%d"%(statistics.median(ctx),pct(ctx,95),statistics.median(ctx)/4,pct(ctx,95)/4))
# 10 QPS for 30 s: launch one request every 100 ms, measure each one's latency
res=[]
def timed(q): 
    try: return call(q)[0]
    except Exception as e: return ("ERR",str(e))
with cf.ThreadPoolExecutor(64) as ex:
    futs=[]; start=time.perf_counter()
    for i in range(300):
        time.sleep(max(0,start+i*0.1-time.perf_counter()))
        futs.append(ex.submit(timed,random.Random(i).choice(Q)))
    res=[f.result() for f in futs]
errs=[r for r in res if isinstance(r,tuple)]; ok=[r for r in res if not isinstance(r,tuple)]
print("10 QPS for 30 s: n=%d errors=%d p50=%.0f p95=%.0f p99=%.0f max=%.0f ms"%(len(res),len(errs),pct(ok,50),pct(ok,95),pct(ok,99),max(ok)))
