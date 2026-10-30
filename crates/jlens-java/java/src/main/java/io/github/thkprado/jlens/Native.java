package io.github.thkprado.jlens;

import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.lang.ref.Cleaner;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileAlreadyExistsException;
import java.nio.file.FileSystemException;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.PosixFileAttributes;
import java.nio.file.attribute.PosixFilePermission;
import java.nio.file.attribute.PosixFilePermissions;
import java.nio.file.attribute.UserPrincipal;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.EnumSet;
import java.util.HexFormat;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.atomic.AtomicReferenceArray;
import java.util.stream.Stream;

/**
 * The C API of {@code jlens-capi} (ADR 0021), loaded at first use. Bytes cross in buffers owned
 * by Rust and known by handle: Java writes a buffer through the address Rust gives, and reads
 * the replies the same way.
 */
// The package's native access is the restricted methods it calls here (JEP 472).
@SuppressWarnings("restricted")
final class Native {
    private Native() {}

    private static final byte REPLY_UNDEFINED = 0;
    private static final byte REPLY_JSON = 1;
    private static final byte REPLY_ERROR = 2;
    private static final byte REPLY_HANDLE = 3;

    /** Frees the native objects that were never closed. */
    static final Cleaner CLEANER = Cleaner.create();

    private static final SymbolLookup LIBRARY = SymbolLookup.libraryLookup(library(), Arena.global());
    private static final ValueLayout.OfLong U64 = ValueLayout.JAVA_LONG;
    private static final ValueLayout.OfLong USIZE = ValueLayout.JAVA_LONG;
    private static final ValueLayout.OfInt U32 = ValueLayout.JAVA_INT;

    private static final MethodHandle BUFFER_NEW = function("jlens_buffer_new", U64, USIZE);
    private static final MethodHandle BUFFER_ADDRESS = critical("jlens_buffer_address", USIZE, U64);
    private static final MethodHandle BUFFER_LEN = critical("jlens_buffer_len", USIZE, U64);
    private static final MethodHandle BUFFER_FREE = critical("jlens_buffer_free", U32, U64);
    private static final MethodHandle COMPILE = function("jlens_compile", U64, U64);
    private static final MethodHandle EXPRESSION_FREE = function("jlens_expression_free", U32, U64);
    private static final MethodHandle READS = function("jlens_reads", U64, U64);
    private static final MethodHandle UNION_READS = function("jlens_union_reads", U64, U64);
    private static final MethodHandle OPTIONS_NEW = function("jlens_options_new", U64, U64);
    private static final MethodHandle OPTIONS_FREE = function("jlens_options_free", U32, U64);
    private static final MethodHandle DOCUMENT_NEW = function("jlens_document_new", U64, U64, U64);
    private static final MethodHandle DOCUMENT_FREE = function("jlens_document_free", U32, U64);
    private static final MethodHandle BINDINGS_NEW =
            function("jlens_bindings_new", U64, U64, U64, ValueLayout.ADDRESS, U64);
    private static final MethodHandle BINDINGS_FREE = function("jlens_bindings_free", U32, U64);
    private static final MethodHandle CALL = function("jlens_call", U64, U64, U32, U64);
    private static final MethodHandle EVALUATE =
            function("jlens_evaluate", U64, U64, U64, U64, U64, U32, U64);
    private static final MethodHandle EVALUATE_DOCUMENT =
            function("jlens_evaluate_document", U64, U64, U64, U64, U64, U32, U64);
    private static final MethodHandle EVALUATE_SOURCE = function("jlens_evaluate_source",
            U64, U64, U64, ValueLayout.ADDRESS, U64, U64, U64, U32, U64);
    private static final MethodHandle EVALUATE_AUDITED =
            function("jlens_evaluate_audited", U64, U64, U64, U64, U64, U64);

    /** The function the C API calls back to read Java objects: {@link Reader#read}. */
    static final MemorySegment READER;
    /** The function the C API calls back to run host functions: {@link Host#call}. */
    static final MemorySegment HOST;

    static {
        try {
            MethodHandle read = MethodHandles.lookup().findStatic(Reader.class, "read",
                    MethodType.methodType(long.class, long.class, int.class, int.class, long.class,
                            long.class, long.class));
            READER = Linker.nativeLinker().upcallStub(read,
                    FunctionDescriptor.of(U64, U64, U32, U32, U64, USIZE, USIZE), Arena.global());
            MethodHandle call = MethodHandles.lookup().findStatic(Host.class, "call",
                    MethodType.methodType(long.class, long.class, int.class, long.class, long.class));
            HOST = Linker.nativeLinker().upcallStub(call,
                    FunctionDescriptor.of(U64, U64, U32, U64, U64), Arena.global());
        } catch (ReflectiveOperationException e) {
            throw new ExceptionInInitializerError(e);
        }
    }

    /** The result format of the evaluations: JSON text. */
    static final int JSON = 0;
    /** The result format of the evaluations: values ({@link Values}). */
    static final int VALUES = 1;

    private static final byte REPLY_VALUES = 4;

    private static MethodHandle function(String name, ValueLayout result, ValueLayout... arguments) {
        return Linker.nativeLinker().downcallHandle(symbol(name), FunctionDescriptor.of(result, arguments));
    }

    /**
     * A downcall that never calls back into Java and returns at once, made without the thread's
     * transition out of Java: the three calls that read a reply cost about 20 ns less (roadmap
     * 0.26 #7). Freeing a large reply, or sweeping the documents this thread converted after one
     * was closed, as any export may, takes longer and holds the garbage collector off meanwhile.
     */
    private static MethodHandle critical(String name, ValueLayout result, ValueLayout... arguments) {
        return Linker.nativeLinker().downcallHandle(symbol(name), FunctionDescriptor.of(result, arguments),
                Linker.Option.critical(false));
    }

    private static MemorySegment symbol(String name) {
        return LIBRARY.find(name)
                .orElseThrow(() -> new UnsatisfiedLinkError("jlens: no symbol " + name));
    }

    // The exports catch every panic, so these calls throw only when a downcall itself fails,
    // which `failed` turns into an Error.

    static long compile(long source) {
        try {
            return (long) COMPILE.invokeExact(source);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static void expressionFree(long expression) {
        try {
            int _ = (int) EXPRESSION_FREE.invokeExact(expression);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static long reads(long expression) {
        try {
            return (long) READS.invokeExact(expression);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static long unionReads(long reads) {
        try {
            return (long) UNION_READS.invokeExact(reads);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static long optionsNew(long options) {
        try {
            return (long) OPTIONS_NEW.invokeExact(options);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static void optionsFree(long options) {
        try {
            int _ = (int) OPTIONS_FREE.invokeExact(options);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static long documentNew(long text, long read) {
        try {
            return (long) DOCUMENT_NEW.invokeExact(text, read);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static void documentFree(long document) {
        try {
            int _ = (int) DOCUMENT_FREE.invokeExact(document);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** Makes bindings: a reply with their handle. Their host functions run through {@link #HOST}. */
    static long bindingsNew(long functions, long variables, long context) {
        try {
            return (long) BINDINGS_NEW.invokeExact(functions, variables, HOST, context);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    static void bindingsFree(long bindings) {
        try {
            int _ = (int) BINDINGS_FREE.invokeExact(bindings);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** Calls argument {@code index}, a function, of host function call {@code frame}: a reply. */
    static long call(long frame, int index, long arguments) {
        try {
            return (long) CALL.invokeExact(frame, index, arguments);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** Evaluates against JSON text in buffer {@code input} (0 for none): the reply's bytes. */
    static byte[] evaluate(long expression, long input, long bindings, long options, int format) {
        int slot = slot();
        Scratch scratch = acquire(slot);
        try {
            return reply((long) EVALUATE.invokeExact(expression, input, bindings, options, format,
                    scratch.handle()), scratch);
        } catch (Throwable e) {
            throw failed(e);
        } finally {
            release(slot, scratch);
        }
    }

    /** Evaluates against a document: the reply's bytes. */
    static byte[] evaluateDocument(long expression, long document, long bindings, long options,
            int format) {
        int slot = slot();
        Scratch scratch = acquire(slot);
        try {
            return reply((long) EVALUATE_DOCUMENT.invokeExact(expression, document, bindings,
                    options, format, scratch.handle()), scratch);
        } catch (Throwable e) {
            throw failed(e);
        } finally {
            release(slot, scratch);
        }
    }

    /**
     * Evaluates against Java objects: {@code input} is the buffer of the input's level, and the C
     * API reads the rest through {@link #READER}. The reply's bytes.
     */
    static byte[] evaluateSource(long expression, long input, long bindings, long options,
            int format) {
        int slot = slot();
        Scratch scratch = acquire(slot);
        try {
            return reply((long) EVALUATE_SOURCE.invokeExact(expression, input, READER, 0L,
                    bindings, options, format, scratch.handle()), scratch);
        } catch (Throwable e) {
            throw failed(e);
        } finally {
            release(slot, scratch);
        }
    }

    /**
     * A buffer the evaluations reply into when the reply fits, so that reading it takes no call
     * (roadmap 0.26 #10); one evaluation uses it at a time.
     */
    private record Scratch(long handle, MemorySegment bytes) {}

    /** The bytes of a scratch buffer. Larger replies come in a buffer of their own. */
    private static final int SCRATCH = 4096;

    /** The slots of the pool of scratch buffers, by thread. */
    private static final int SLOTS = 64;

    /** Slots this many entries apart, so that threads on different slots share no cache line. */
    private static final int STRIDE = 16;

    /**
     * The scratch buffers not in use. An evaluation takes its thread's slot's, or makes one when
     * the slot is empty (another thread on it, or an evaluation of a host function's on this
     * one), and puts it back, or frees it when the slot was filled meanwhile. The buffers kept,
     * at most 256 KiB, live as long as the process.
     */
    private static final AtomicReferenceArray<Scratch> POOL = new AtomicReferenceArray<>(SLOTS * STRIDE);

    private static int slot() {
        return ((int) Thread.currentThread().threadId() & (SLOTS - 1)) * STRIDE;
    }

    private static Scratch acquire(int slot) {
        Scratch scratch = POOL.getAndSet(slot, null);
        if (scratch != null) return scratch;
        long handle = bufferNew(SCRATCH);
        return new Scratch(handle, bufferBytes(handle, SCRATCH));
    }

    private static void release(int slot, Scratch scratch) {
        if (!POOL.compareAndSet(slot, null, scratch)) free(scratch.handle());
    }

    /**
     * The bytes of the reply an evaluation returned: in {@code scratch} when the low half of
     * {@code replied} is 0 and the high half its length, in a buffer of its own otherwise.
     */
    private static byte[] reply(long replied, Scratch scratch) {
        if ((int) replied != 0 || replied == 0) return take(replied);
        byte[] bytes = new byte[(int) (replied >>> 32)];
        MemorySegment.copy(scratch.bytes(), ValueLayout.JAVA_BYTE, 0, bytes, 0, bytes.length);
        return bytes;
    }

    /**
     * Evaluates against the values in buffer {@code input} (0 for none), recording an audit with
     * the options in buffer {@code audit} (0 for the defaults).
     */
    static long evaluateAudited(long expression, long input, long bindings, long options,
            long audit) {
        try {
            return (long) EVALUATE_AUDITED.invokeExact(expression, input, bindings, options, audit);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** A new buffer of {@code size} bytes: its handle, which the caller frees. */
    static long bufferNew(long size) {
        try {
            long buffer = (long) BUFFER_NEW.invokeExact(size);
            if (buffer == 0) throw new OutOfMemoryError("jlens: no buffer of " + size + " bytes");
            return buffer;
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** The {@code size} bytes of buffer {@code buffer}, valid until it is freed. */
    static MemorySegment bufferBytes(long buffer, long size) {
        try {
            long address = (long) BUFFER_ADDRESS.invokeExact(buffer);
            return MemorySegment.ofAddress(address).reinterpret(size);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** A copy of the bytes of buffer {@code buffer}, which stays live. */
    static byte[] bytes(long buffer) {
        try {
            long address = (long) BUFFER_ADDRESS.invokeExact(buffer);
            long len = (long) BUFFER_LEN.invokeExact(buffer);
            return MemorySegment.ofAddress(address).reinterpret(len).toArray(ValueLayout.JAVA_BYTE);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** A copy of the {@code length} bytes at {@code address}, which the C API gave. */
    static byte[] bytesAt(long address, long length) {
        return MemorySegment.ofAddress(address).reinterpret(length).toArray(ValueLayout.JAVA_BYTE);
    }

    /** A new buffer holding {@code bytes}: its handle, which the caller frees. */
    static long buffer(byte[] bytes) {
        try {
            long buffer = (long) BUFFER_NEW.invokeExact((long) bytes.length);
            if (buffer == 0) throw new OutOfMemoryError("jlens: no buffer of " + bytes.length + " bytes");
            long address = (long) BUFFER_ADDRESS.invokeExact(buffer);
            MemorySegment.copy(bytes, 0, MemorySegment.ofAddress(address).reinterpret(bytes.length),
                    ValueLayout.JAVA_BYTE, 0, bytes.length);
            return buffer;
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /** A new buffer holding {@code text} in UTF-8. */
    static long buffer(String text) {
        return buffer(text.getBytes(StandardCharsets.UTF_8));
    }

    static void free(long buffer) {
        try {
            int _ = (int) BUFFER_FREE.invokeExact(buffer);
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /**
     * Reads and frees reply {@code reply}: the JSON text it holds, or {@code null} when it is
     * undefined. A reply holding an error throws it.
     */
    static String json(long reply) {
        return json(take(reply));
    }

    /**
     * The JSON text that reply {@code bytes} holds, or {@code null} when it is undefined. A reply
     * holding an error throws it.
     */
    static String json(byte[] bytes) {
        return switch (bytes[0]) {
            case REPLY_UNDEFINED -> null;
            case REPLY_JSON -> new String(bytes, 1, bytes.length - 1, StandardCharsets.UTF_8);
            case REPLY_ERROR -> throw error(bytes);
            default -> throw unexpected(bytes);
        };
    }

    /**
     * Reads and frees reply {@code reply}: the result it holds as values, decoded, or {@code null}
     * when it is undefined. A reply holding an error throws it.
     */
    static Object values(long reply) {
        return values(reply, null);
    }

    /**
     * Reads and frees reply {@code reply} as {@link #values(long)} does, its arrays and objects of
     * the input taken from {@code input}, by their numbers.
     */
    static Object values(long reply, Object[] input) {
        return values(take(reply), input);
    }

    /**
     * The result that reply {@code bytes} holds as values, decoded, its arrays and objects of the
     * input taken from {@code input}; {@code null} when it is undefined. A reply holding an error
     * throws it.
     */
    static Object values(byte[] bytes, Object[] input) {
        return switch (bytes[0]) {
            case REPLY_UNDEFINED -> null;
            case REPLY_VALUES -> Values.decode(bytes, 1, input);
            case REPLY_ERROR -> throw error(bytes);
            default -> throw unexpected(bytes);
        };
    }

    /** Reads and frees reply {@code reply}: the handle it holds. A reply holding an error throws it. */
    static long handle(long reply) {
        byte[] bytes = take(reply);
        return switch (bytes[0]) {
            case REPLY_HANDLE -> {
                long handle = 0;
                for (int i = 8; i >= 1; i--) handle = (handle << 8) | (bytes[i] & 0xff);
                yield handle;
            }
            case REPLY_ERROR -> throw error(bytes);
            default -> throw unexpected(bytes);
        };
    }

    private static byte[] take(long reply) {
        if (reply == 0) throw new OutOfMemoryError("jlens: no buffer for a reply");
        try {
            long address = (long) BUFFER_ADDRESS.invokeExact(reply);
            long len = (long) BUFFER_LEN.invokeExact(reply);
            byte[] bytes = MemorySegment.ofAddress(address).reinterpret(len).toArray(ValueLayout.JAVA_BYTE);
            int _ = (int) BUFFER_FREE.invokeExact(reply);
            return bytes;
        } catch (Throwable e) {
            throw failed(e);
        }
    }

    /**
     * The exception of an error reply. {@code U2002}, a handle that names no live object, means
     * that a Java object was closed while in use.
     */
    private static RuntimeException error(byte[] bytes) {
        JlensException error = JlensException.fromEnvelope(
                new String(bytes, 1, bytes.length - 1, StandardCharsets.UTF_8));
        if (error.code().equals("U2002")) {
            return new IllegalStateException("jlens: the " + error.token().orElse("object")
                    + " was closed", error);
        }
        return error;
    }

    private static IllegalStateException unexpected(byte[] bytes) {
        return new IllegalStateException("jlens: unexpected reply " + bytes[0]);
    }

    private static RuntimeException failed(Throwable e) {
        return switch (e) {
            case RuntimeException runtime -> runtime;
            case Error error -> throw error;
            default -> new IllegalStateException("jlens: a native call failed", e);
        };
    }

    /**
     * The path of the native library: the system property {@code jlens.library} when it is set,
     * or the host platform's library in the JAR, copied to the temporary directory once per user
     * and content. Several JVMs can start at once: each writes its own file, then moves it in
     * place. The directory must be this user's alone, or another user could put a library of
     * their own there for this one to load.
     */
    private static Path library() {
        String given = System.getProperty("jlens.library");
        if (given != null) return Path.of(given);
        String name = System.mapLibraryName("jlens_capi");
        String resource = "native/" + platform() + "/" + name;
        byte[] bytes;
        try (InputStream in = Native.class.getResourceAsStream(resource)) {
            if (in == null) {
                throw new UnsatisfiedLinkError("jlens: no native library for " + platform()
                        + " in the JAR (" + resource + ")");
            }
            bytes = in.readAllBytes();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        try {
            Path temporary = Path.of(System.getProperty("java.io.tmpdir"));
            String user = System.getProperty("user.name", "").replaceAll("[^A-Za-z0-9._-]", "_");
            Path directory = temporary.resolve("jlens-" + user + "-" + digest(bytes));
            own(temporary, directory);
            Path library = directory.resolve(name);
            if (Files.isRegularFile(library, LinkOption.NOFOLLOW_LINKS)
                    && Files.size(library) == bytes.length) {
                return library;
            }
            Path part = Files.createTempFile(directory, name, ".part");
            Files.write(part, bytes);
            try {
                Files.move(part, library, StandardCopyOption.ATOMIC_MOVE);
            } catch (FileSystemException e) {
                // On Windows, a file another JVM moved in first and holds open cannot be
                // replaced; it has the same bytes.
                Files.deleteIfExists(part);
                if (!Files.isRegularFile(library, LinkOption.NOFOLLOW_LINKS)
                        || Files.size(library) != bytes.length) {
                    throw e;
                }
            }
            return library;
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    /**
     * Makes {@code directory}, only its owner allowed in, or checks that it is so: a directory
     * of this user's, not a link, that no one else may write to. Where files have no POSIX
     * permissions (Windows), the temporary directory is the user's own.
     */
    private static void own(Path temporary, Path directory) throws IOException {
        if (!FileSystems.getDefault().supportedFileAttributeViews().contains("posix")) {
            Files.createDirectories(directory);
            return;
        }
        try {
            Files.createDirectory(directory,
                    PosixFilePermissions.asFileAttribute(PosixFilePermissions.fromString("rwx------")));
            return;
        } catch (FileAlreadyExistsException e) {
            // Made before, by this JVM's user or not: checked below.
        }
        Path probe = Files.createTempFile(temporary, "jlens-", ".owner");
        UserPrincipal self;
        try {
            self = Files.getOwner(probe);
        } finally {
            Files.deleteIfExists(probe);
        }
        PosixFileAttributes attributes =
                Files.readAttributes(directory, PosixFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
        Set<PosixFilePermission> others = EnumSet.of(PosixFilePermission.GROUP_WRITE,
                PosixFilePermission.OTHERS_WRITE);
        others.retainAll(attributes.permissions());
        if (!attributes.isDirectory() || !attributes.owner().equals(self) || !others.isEmpty()) {
            throw new UnsatisfiedLinkError("jlens: " + directory + " is not a directory of this"
                    + " user's alone; remove it, or set java.io.tmpdir or jlens.library");
        }
    }

    /**
     * The host platform, as the JAR's directories name it: {@code windows-x86_64},
     * {@code linux-x86_64-musl} and so on.
     */
    static String platform() {
        String os = System.getProperty("os.name").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch").toLowerCase(Locale.ROOT);
        String system = os.startsWith("windows") ? "windows"
                : os.startsWith("mac") ? "macos"
                : os.startsWith("linux") ? "linux"
                : os.replace(' ', '_');
        String machine = switch (arch) {
            case "amd64", "x86_64" -> "x86_64";
            case "aarch64", "arm64" -> "aarch64";
            default -> arch;
        };
        return system + "-" + machine + (system.equals("linux") && musl() ? "-musl" : "");
    }

    /**
     * Whether this JVM runs on musl, as on Alpine Linux: musl's dynamic loader is mapped into the
     * process. Files on disk do not tell, as a glibc system can have musl installed too.
     */
    private static boolean musl() {
        try (Stream<String> maps =
                Files.lines(Path.of("/proc/self/maps"), StandardCharsets.ISO_8859_1)) {
            return maps.anyMatch(line -> line.contains("/ld-musl-"));
        } catch (IOException | UncheckedIOException e) {
            return false;
        }
    }

    private static String digest(byte[] bytes) {
        try {
            byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes);
            return HexFormat.of().formatHex(digest, 0, 8);
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }
}
