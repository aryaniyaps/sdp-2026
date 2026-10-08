"""Dependency-free Ollama inference for a fully rendered memory task prompt."""
import argparse, json, urllib.request
import prompt, structured

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--model',default='mem-extractor')
    p.add_argument('--host',default='http://127.0.0.1:11434')
    p.add_argument('--prompt-file',required=True)
    a=p.parse_args()
    with open(a.prompt_file) as f: user=f.read()
    user,schema,paired=structured.prepare(user)
    payload={'model':a.model,'system':prompt.SYSTEM,'prompt':user,'format':schema,'think':False,'stream':False,'options':{'temperature':0,'num_ctx':12288}}
    req=urllib.request.Request(a.host.rstrip('/')+'/api/generate',data=json.dumps(payload).encode(),headers={'Content-Type':'application/json'})
    with urllib.request.urlopen(req,timeout=600) as response: result=json.load(response)
    if result.get('done_reason')=='length':raise RuntimeError('Model output was truncated; no canonical result returned')
    print(json.dumps(structured.canonicalize(json.loads(result['response']),paired),ensure_ascii=False,indent=2))

if __name__=='__main__': main()
