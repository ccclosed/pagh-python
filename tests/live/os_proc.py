import os

pid = os.fork()
if pid == 0:
    os.execv("/mnt/pythonc", ["/mnt/pythonc", "/mnt/hello.py"])
    print("exec failed")
else:
    child, status = os.waitpid(pid, 0)
    print("child:", child == pid, "status:", status)
    assert child == pid
    assert status == 0
print("proc ok")
