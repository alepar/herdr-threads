import socket, sys
for target in sys.argv[1:]:
    try:
        if target.startswith('tcp:'):
            h,p=target[4:].rsplit(':',1); s=socket.create_connection((h,int(p)),timeout=5)
        else:
            s=socket.socket(socket.AF_UNIX); s.settimeout(5); s.connect(target)
        print(target, 'CONNECTED'); s.close()
    except Exception as e:
        print(target, 'ERR', type(e).__name__, e)
