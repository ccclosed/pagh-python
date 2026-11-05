import os

base = "/mnt/osdemo"
os.mkdir(base)
os.makedirs(base + "/nested/deep")
path = base + "/file.txt"
writer = open(path, "w")
writer.write("hello os")
writer.close()

listing = sorted(os.listdir(base))
print("listdir:", listing)
assert listing == ["file.txt", "nested"]

info = os.stat(path)
print("stat:", info.st_size, info.st_kind, info.st_isfile, info.st_isdir)
assert info.st_size == 8
assert info.st_kind == "file"
assert info.st_isfile
assert not info.st_isdir

os.rename(path, base + "/renamed.txt")
assert os.stat(base + "/renamed.txt").st_size == 8
print("rename:", sorted(os.listdir(base)))

os.remove(base + "/renamed.txt")
os.rmdir(base + "/nested/deep")
os.rmdir(base + "/nested")
os.rmdir(base)
removed = False
try:
    os.stat(base)
except FileNotFoundError:
    removed = True
assert removed

cwd_info = os.stat(os.getcwd())
print("cwd stat:", cwd_info.st_kind, cwd_info.st_isdir)
assert cwd_info.st_kind == "dir"
assert cwd_info.st_isdir

print("pid:", os.getpid() > 0, os.getppid() >= 0)
print("cwd:", os.getcwd())
assert os.getpid() > 0
assert os.getppid() >= 0
assert len(os.getcwd()) > 0
assert "PWD" in os.environ
assert len(os.environ["PWD"]) > 0
print("os ok")
