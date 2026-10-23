"""Minimal stand-in for the Ollama REST API, used by the end-to-end test.

Usage: python3 mock_ollama.py <port>
"""
import json, time, sys
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from datetime import datetime, timezone, timedelta
MODELS={"llama3.2:3b":2019393189,"qwen3:8b":5225387923,"gemma3:12b":8149190253}
RUNNING={"llama3.2:3b"}
# Canned answers for the prompt used by scripts/screenshots.sh; anything else is echoed.
ANSWERS={
 "llama3.2:3b":"It limits how many tokens the model can see at once: your prompt, the earlier conversation and the reply it is writing. Anything beyond that is dropped, so long chats or documents need a larger window, which also uses more memory.",
 "qwen3:8b":"The context window caps the total number of tokens, input plus output, that the model can attend to in a single request. Once a conversation outgrows it, the oldest messages are truncated and the model no longer remembers them.",
 "gemma3:12b":"It bounds the amount of text, measured in tokens, that fits in the model's working memory for one request. A bigger window lets it follow longer conversations, at the cost of more VRAM and slower prompt processing.",
}
def det(): return {"format":"gguf","family":"llama","parameter_size":"3.2B","quantization_level":"Q4_K_M"}
class H(BaseHTTPRequestHandler):
    def log_message(self,*a): sys.stderr.write("[GIN] %s %s\n"%(self.command,self.path))
    def j(self,o,code=200):
        b=json.dumps(o).encode(); self.send_response(code); self.send_header("Content-Type","application/json"); self.send_header("Content-Length",str(len(b))); self.end_headers(); self.wfile.write(b)
    def body(self):
        n=int(self.headers.get("Content-Length") or 0); return json.loads(self.rfile.read(n) or b"{}")
    def do_GET(self):
        if "Authorization" in self.headers: return self.j({"error":"auth header leaked to upstream"},500)
        if self.path=="/api/version": return self.j({"version":"0.12.6-mock"})
        if self.path=="/api/tags": return self.j({"models":[{"name":k,"model":k,"modified_at":"2026-10-01T10:00:00Z","size":v,"digest":"abc","details":det()} for k,v in MODELS.items()]})
        if self.path=="/api/ps":
            exp=(datetime.now(timezone.utc)+timedelta(minutes=4)).isoformat()
            return self.j({"models":[{"name":k,"model":k,"size":MODELS[k]+500_000_000,"size_vram":int((MODELS[k]+500_000_000)*0.7),"expires_at":exp,"context_length":8192,"details":det()} for k in RUNNING]})
        self.j({"error":"not found"},404)
    def do_DELETE(self):
        b=self.body(); MODELS.pop(b["model"],None); RUNNING.discard(b["model"]); self.j({})
    def do_POST(self):
        b=self.body()
        if self.path=="/api/show": return self.j({"modelfile":"FROM x","parameters":"temperature 0.7","template":"{{ .Prompt }}","details":det(),"model_info":{"llama.context_length":131072},"capabilities":["completion"]})
        if self.path=="/api/generate":
            if b.get("keep_alive")==0: RUNNING.discard(b["model"])
            else: RUNNING.add(b["model"])
            return self.j({"done":True,"total_duration":3_100_000_000,"load_duration":900_000_000,"prompt_eval_count":24,"prompt_eval_duration":120_000_000,"eval_count":256,"eval_duration":2_000_000_000})
        if self.path=="/api/chat":
            if b["model"] not in MODELS: return self.j({"error":"model '%s' not found"%b["model"]},404)
            RUNNING.add(b["model"]); self.send_response(200); self.send_header("Content-Type","application/x-ndjson"); self.end_headers()
            q=b["messages"][-1]["content"]
            words=(ANSWERS.get(b["model"],ANSWERS["llama3.2:3b"]) if "context window" in q else "You said: "+q).split(" ")
            time.sleep(0.4)
            for w in words:
                self.wfile.write((json.dumps({"model":b["model"],"message":{"role":"assistant","content":w+" "},"done":False})+"\n").encode()); self.wfile.flush(); time.sleep(0.05)
            self.wfile.write((json.dumps({"model":b["model"],"message":{"role":"assistant","content":""},"done":True,"total_duration":1_500_000_000,"load_duration":0,"prompt_eval_count":12,"prompt_eval_duration":60_000_000,"eval_count":len(words),"eval_duration":len(words)*50_000_000})+"\n").encode()); return
        if self.path=="/api/pull":
            self.send_response(200); self.send_header("Content-Type","application/x-ndjson"); self.end_headers()
            for i in range(0,101,10):
                self.wfile.write((json.dumps({"status":"pulling sha256:ff","digest":"ff","total":1000,"completed":i*10})+"\n").encode()); self.wfile.flush(); time.sleep(0.2)
            self.wfile.write(b'{"status":"success"}\n'); MODELS[b["model"]]=1000; return
        self.j({"error":"not found"},404)
ThreadingHTTPServer(("127.0.0.1",int(sys.argv[1])),H).serve_forever()
