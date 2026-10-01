import socket, sys, os, time, threading
u=socket.socket(socket.AF_UNIX); p=sys.argv[1]
try: os.unlink(p)
except FileNotFoundError: pass
u.bind(p); u.listen(8)
t=socket.socket(); t.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR,1); t.bind(('127.0.0.1',47812)); t.listen(8)
def acc(s):
    while True:
        c,_=s.accept(); c.close()
for s in (u,t): threading.Thread(target=acc,args=(s,),daemon=True).start()
time.sleep(600)
