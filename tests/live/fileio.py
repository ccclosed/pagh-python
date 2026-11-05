path = "/mnt/py.txt"
writer = open(path, "w")
writer.write("first line\n")
writer.writelines(["second line\n", "third line\n"])
writer.close()

reader = open(path, "r")
lines = []
for line in reader:
    lines.append(line.strip())
reader.close()
print("file lines:", lines)

reader = open(path, "r")
print("readline:", reader.readline().strip())
print("rest:", reader.read().replace("\n", "|"))
reader.close()

out = open("/mnt/print.txt", "w")
print("print to file", file=out)
out.close()
