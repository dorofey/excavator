#!/usr/bin/env python3
"""Loopback-only deterministic S3 failure proxy for disposable fixtures."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import urllib.request, urllib.error, threading, json
lock=threading.Lock(); counts={}
class Proxy(BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_GET(self):self.relay()
    def do_HEAD(self):self.relay()
    def do_POST(self):self.relay()
    def do_PUT(self):self.relay()
    def do_DELETE(self):self.relay()
    def relay(self):
        if self.path=='/__fixture_counts':
            with lock:body=json.dumps(counts).encode()
            self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        with lock:
            key=self.command+' '+self.path;counts[key]=counts.get(key,0)+1;count=counts[key]
        body=self.rfile.read(int(self.headers.get('Content-Length','0')))
        fail=('retry-prefix' in self.path and self.command=='GET' and count<=2) or ('fail-upload.bin' in self.path and self.command=='PUT' and 'partNumber=' in self.path)
        if fail:
            response=b'<Error><Code>SlowDown</Code><Message>Disposable fixture fault</Message></Error>'
            self.send_response(503);self.send_header('Content-Length',str(len(response)));self.end_headers();self.wfile.write(response);return
        headers={k:v for k,v in self.headers.items() if k.lower() not in ('host','connection')}
        req=urllib.request.Request('http://127.0.0.1:22222'+self.path,data=body if body or self.command in ('POST','PUT') else None,headers=headers,method=self.command)
        try:reply=urllib.request.urlopen(req,timeout=20)
        except urllib.error.HTTPError as error:reply=error
        data=reply.read();self.send_response(reply.status)
        for name,value in reply.headers.items():
            if name.lower() not in ('connection','transfer-encoding','server','date'):self.send_header(name,value)
        self.end_headers()
        if self.command!='HEAD':
            if 'partial.txt' in self.path and self.command=='GET':self.wfile.write(data[:max(1,len(data)//2)]);self.wfile.flush();self.close_connection=True
            else:self.wfile.write(data)
ThreadingHTTPServer(('127.0.0.1',22223),Proxy).serve_forever()
