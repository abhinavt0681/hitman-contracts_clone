// Headless export after auto-analysis: decompiled C for every function, a function index,
// and which functions reference which strings. Output directory is the script argument.
//@category HitmanClone
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import ghidra.program.model.data.StringDataInstance;
import java.io.*;
import java.util.*;

public class ExportAll extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        File out = new File(args.length > 0 ? args[0] : "ghidra-out");
        File cdir = new File(out, "decompiled");
        cdir.mkdirs();

        // String → referencing functions.
        try (PrintWriter w = new PrintWriter(new FileWriter(new File(out, "strings_xrefs.tsv")))) {
            w.println("string_addr\tfunctions\tstring");
            DataIterator it = currentProgram.getListing().getDefinedData(true);
            while (it.hasNext() && !monitor.isCancelled()) {
                Data d = it.next();
                if (!d.hasStringValue()) continue;
                String s = StringDataInstance.getStringDataInstance(d).getStringValue();
                if (s == null) continue;
                Set<String> fns = new TreeSet<>();
                for (Reference r : getReferencesTo(d.getAddress())) {
                    Function f = getFunctionContaining(r.getFromAddress());
                    if (f != null) fns.add(f.getEntryPoint() + ":" + f.getName());
                }
                w.println(d.getAddress() + "\t" + String.join(",", fns) + "\t" + s.replace("\t", "\\t").replace("\n", "\\n"));
            }
        }

        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        FunctionIterator fit = currentProgram.getFunctionManager().getFunctions(true);
        int n = 0;
        PrintWriter chunk = null;
        long chunkKey = -1;
        try (PrintWriter idx = new PrintWriter(new FileWriter(new File(out, "functions.tsv")))) {
            idx.println("entry\tname\tsize\tcallers\tcallees");
            while (fit.hasNext() && !monitor.isCancelled()) {
                Function f = fit.next();
                long a = f.getEntryPoint().getOffset();
                idx.println(f.getEntryPoint() + "\t" + f.getName() + "\t" + f.getBody().getNumAddresses() + "\t"
                    + f.getCallingFunctions(monitor).size() + "\t" + f.getCalledFunctions(monitor).size());
                // One file per 64 KB of address space keeps files greppable but not huge.
                long key = a >> 16;
                if (key != chunkKey) {
                    if (chunk != null) chunk.close();
                    chunk = new PrintWriter(new FileWriter(new File(cdir, String.format("%08x.c", key << 16))));
                    chunkKey = key;
                }
                DecompileResults res = di.decompileFunction(f, 60, monitor);
                chunk.println("// ==== " + f.getEntryPoint() + " " + f.getName());
                if (res != null && res.decompileCompleted()) {
                    chunk.println(res.getDecompiledFunction().getC());
                } else {
                    chunk.println("// decompile failed: " + (res == null ? "null" : res.getErrorMessage()));
                }
                if (++n % 500 == 0) println("decompiled " + n);
            }
        }
        if (chunk != null) chunk.close();
        println("done: " + n + " functions");
    }
}
