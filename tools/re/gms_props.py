"""Dump tagged property blocks of GMS entities by name: python3 -I gms_props.py RAW BUF NAME..."""
import struct, sys

def main(raw, buf, wanted):
    b = open(raw, 'rb').read(); names = open(buf, 'rb').read()
    u = lambda o: struct.unpack_from('<I', b, o)[0]
    t = u(0); n = u(t)
    offs = [(u(t + 4 + i * 8) & 0xffffff) * 4 for i in range(n)]
    for i, e in enumerate(offs):
        nm = names[u(e):names.index(b'\0', u(e))].decode('latin1')
        if not any(w.lower() in nm.lower() for w in wanted):
            continue
        at = u(e + 0x20)
        print(f'[{i}] {nm} type={u(e+0x10):#x} props@{at:#x}')
        if not 0 < at < len(b):
            continue
        size = u(at)
        p, end = at + 4, at + 4 + min(size, 600)
        out = []
        while p < end:
            tag = b[p]
            if tag in (0x02, 0x82, 0x42, 0xc2):
                out.append(f'f{struct.unpack_from("<f", b, p+1)[0]:.4g}'); p += 5
            elif tag in (0x03, 0x83, 0x43, 0xc3, 0x8b, 0x06, 0x86, 0x46, 0x08, 0x88):
                v = u(p + 1); out.append(f'{tag:02x}:{v:#x}' if v > 0xffff else f'{tag:02x}:{v}'); p += 5
            elif tag in (0x81, 0x41, 0x01, 0xc1):
                out.append(f'd{struct.unpack_from("<d", b, p+1)[0]:.4g}'); p += 9
            elif tag == 0x89:
                sz = u(p + 1)
                if p + 1 + sz > len(b): break
                refs = [u(p + 5 + 4 * k) for k in range((sz - 4) // 4)]
                out.append(f'refs{[(r & 0xffffff) - 1 for r in refs]}'); p += 1 + sz
            elif tag in (0x04, 0x84, 0x85):
                sz = u(p + 1); out.append(repr(b[p+5:p+5+sz].split(b'\0')[0].decode('latin1'))); p += 5 + sz
            elif tag == 0x00:
                p += 1
            else:
                out.append(f'?{tag:02x}'); p += 1
        print('   ', ' '.join(out)[:900])

main(sys.argv[1], sys.argv[2], sys.argv[3:])
