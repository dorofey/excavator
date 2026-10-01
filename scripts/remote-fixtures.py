#!/usr/bin/env python3
"""Disposable loopback SFTP/FTPS/S3 fixtures. Install deps in a temporary venv.
Not a production server. Uses only public dummy fixture credentials.
"""
import argparse, os, pathlib, signal, subprocess, threading, time
import paramiko
from paramiko import SFTPServerInterface, SFTPAttributes, SFTPHandle
from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import TLS_FTPHandler
from pyftpdlib.servers import FTPServer
import socket

parser=argparse.ArgumentParser()
parser.add_argument('--root',required=True)
args=parser.parse_args()
root=pathlib.Path(args.root).resolve(); root.mkdir(parents=True,exist_ok=True)
for name in ('sftp','ftps'):
    folder=root/name; folder.mkdir(exist_ok=True)
    (folder/'hello.txt').write_text('remote fixture bytes\n')
    (folder/'empty').mkdir(exist_ok=True)
key_path=root/'host.key'
if key_path.exists(): host_key=paramiko.RSAKey.from_private_key_file(str(key_path))
else:
    host_key=paramiko.RSAKey.generate(2048); host_key.write_private_key_file(str(key_path)); key_path.chmod(0o600)
class Auth(paramiko.ServerInterface):
    def check_auth_password(self, username,password):
        return paramiko.AUTH_SUCCESSFUL if username=='fixture' and password=='fixture-password' else paramiko.AUTH_FAILED
    def get_allowed_auths(self, username): return 'password'
    def check_channel_request(self,kind,chanid): return paramiko.OPEN_SUCCEEDED if kind=='session' else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
class Handle(SFTPHandle):
    def stat(self):
        f=getattr(self,'readfile',None) or getattr(self,'writefile',None)
        try:return SFTPAttributes.from_stat(os.fstat(f.fileno()))
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
class Sftp(SFTPServerInterface):
    def path(self,path):
        result=(root/'sftp'/path.lstrip('/')).resolve()
        if not result.is_relative_to(root/'sftp'): raise PermissionError('Outside fixture')
        return result
    def list_folder(self,path):
        try:
            result=[]
            for child in self.path(path).iterdir():
                attr=SFTPAttributes.from_stat(child.lstat()); attr.filename=child.name; result.append(attr)
            return result
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
    def stat(self,path):
        try:return SFTPAttributes.from_stat(self.path(path).stat())
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
    def lstat(self,path):
        try:
            if path in ('','/') :return SFTPAttributes.from_stat((root/'sftp').lstat())
            raw=root/'sftp'/path.lstrip('/')
            parent=raw.parent.resolve()
            if not parent.is_relative_to(root/'sftp'):return paramiko.SFTP_PERMISSION_DENIED
            return SFTPAttributes.from_stat((parent/raw.name).lstat())
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
    def open(self,path,flags,attr):
        try:
            fd=os.open(self.path(path),flags,0o600)
            mode='r+b' if flags&os.O_RDWR else ('wb' if flags&os.O_WRONLY else 'rb')
            f=os.fdopen(fd,mode); handle=Handle(flags)
            if flags&os.O_WRONLY or flags&os.O_RDWR:handle.writefile=f
            if not flags&os.O_WRONLY:handle.readfile=f
            return handle
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
    def remove(self,path):
        try:self.path(path).unlink(); return paramiko.SFTP_OK
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
    def mkdir(self,path,attr):
        try:self.path(path).mkdir(); return paramiko.SFTP_OK
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
    def rmdir(self,path):
        try:self.path(path).rmdir(); return paramiko.SFTP_OK
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
    def rename(self,old,new):
        try:
            if self.path(new).exists():return paramiko.SFTP_FAILURE
            self.path(old).rename(self.path(new));return paramiko.SFTP_OK
        except OSError as e:return paramiko.SFTPServer.convert_errno(e.errno)
transports=[]
listener=socket.socket();listener.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);listener.bind(('127.0.0.1',22220));listener.listen(10)
def ssh_loop():
    while True:
        sock,_=listener.accept()
        def serve(sock):
            t=paramiko.Transport(sock);transports.append(t);t.add_server_key(host_key)
            t.set_subsystem_handler('sftp',paramiko.SFTPServer,Sftp)
            try:t.start_server(server=Auth());t.join()
            except Exception: t.close()
        threading.Thread(target=serve,args=(sock,),daemon=True).start()
threading.Thread(target=ssh_loop,daemon=True).start()
cert=root/'tls.pem';private=root/'tls.key';ca=root/'ca.pem';ca_key=root/'ca.key'
if not cert.exists():
    quiet=dict(check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(ca_key),'-out',str(ca),'-days','2','-subj','/CN=Excavator Disposable Fixture CA','-addext','basicConstraints=critical,CA:TRUE','-addext','keyUsage=critical,keyCertSign,cRLSign'],**quiet)
    csr=root/'tls.csr';ext=root/'tls.ext'
    ext.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n')
    subprocess.run(['openssl','req','-new','-newkey','rsa:2048','-nodes','-keyout',str(private),'-out',str(csr),'-subj','/CN=localhost'],**quiet)
    subprocess.run(['openssl','x509','-req','-in',str(csr),'-CA',str(ca),'-CAkey',str(ca_key),'-CAcreateserial','-out',str(cert),'-days','2','-extfile',str(ext)],**quiet)
    private.chmod(0o600);ca_key.chmod(0o600)
auth=DummyAuthorizer();auth.add_user('fixture','fixture-password',str(root/'ftps'),perm='elradfmwMT')
class Ftps(TLS_FTPHandler):
    certfile=str(cert);keyfile=str(private);authorizer=auth;tls_control_required=True;tls_data_required=True;passive_ports=range(22300,22320)
ftp=FTPServer(('127.0.0.1',22221),Ftps)
threading.Thread(target=ftp.serve_forever,daemon=True).start()
moto=subprocess.Popen([str(pathlib.Path(os.sys.executable).parent/'moto_server'),'-H','127.0.0.1','-p','22222'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
def stop(*_):
    moto.terminate();ftp.close_all();listener.close()
    for t in transports:t.close()
    raise SystemExit
signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
# Seed only the disposable bucket; preserve it across server restarts.
import boto3
s3=boto3.client('s3',endpoint_url='http://127.0.0.1:22222',aws_access_key_id='fixture',aws_secret_access_key='fixture',region_name='us-east-1')
for attempt in range(40):
    try:
        buckets=s3.list_buckets();break
    except Exception:time.sleep(0.25)
else:stop()
if not any(bucket['Name']=='excavator-fixture' for bucket in buckets['Buckets']):
    s3.create_bucket(Bucket='excavator-fixture')
    s3.put_bucket_versioning(Bucket='excavator-fixture',VersioningConfiguration={'Status':'Enabled'})
    s3.put_object(Bucket='excavator-fixture',Key='hello.txt',Body=b'remote fixture bytes\n')
    for i in range(1005):s3.put_object(Bucket='excavator-fixture',Key=f'pages/page-{i:04}.txt',Body=b'page')
    s3.put_object(Bucket='excavator-fixture',Key='prefix/nested.txt',Body=b'nested')
    s3.put_object(Bucket='excavator-fixture',Key='partial.txt',Body=b'x'*1048576)
    s3.put_object(Bucket='excavator-fixture',Key='retry-prefix/hello.txt',Body=b'retry')
print(f'Loopback fixtures ready: SFTP 22220, FTPS 22221, S3 22222; CA {ca}',flush=True)
while True:time.sleep(1)
