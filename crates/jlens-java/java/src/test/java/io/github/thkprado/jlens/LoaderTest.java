package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.PosixFilePermissions;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Stream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/** The loading of the native library from the JAR (0.25 #12). */
class LoaderTest {
    private static final String LIBRARY = System.mapLibraryName("jlens_capi");

    /**
     * The JAR holds a library for the platform the loader detects. CI names the platform each
     * target must be detected as in {@code JLENS_TEST_PLATFORM}, musl included.
     */
    @Test
    void theDetectedPlatformIsInTheJar() {
        String platform = Native.platform();
        String expected = System.getenv("JLENS_TEST_PLATFORM");
        if (expected != null) assertEquals(expected, platform);
        assertNotNull(Native.class.getResource("native/" + platform + "/" + LIBRARY), platform);
    }

    /** JVMs that start at once share one copy of the library, and leave no partial file. */
    @Test
    void severalJvmsExtractTheLibraryAtOnce(@TempDir Path temporary) throws Exception {
        List<Process> processes = new ArrayList<>();
        List<Path> outputs = new ArrayList<>();
        for (int i = 0; i < 8; i++) {
            Path output = temporary.resolve("output-" + i + ".txt");
            outputs.add(output);
            processes.add(probe(temporary, output).start());
        }
        for (int i = 0; i < processes.size(); i++) {
            assertEquals(0, processes.get(i).waitFor(), Files.readString(outputs.get(i)));
        }
        Path directory = extracted(temporary);
        try (Stream<Path> files = Files.list(directory)) {
            assertEquals(List.of(LIBRARY), files.map(f -> f.getFileName().toString()).toList());
        }
    }

    /** Where files have POSIX permissions, a directory others may write to is refused. */
    @Test
    void aDirectoryOthersMayWriteToIsRefused(@TempDir Path temporary) throws Exception {
        if (!FileSystems.getDefault().supportedFileAttributeViews().contains("posix")) return;
        Path output = temporary.resolve("output.txt");
        assertEquals(0, probe(temporary, output).start().waitFor(), Files.readString(output));
        Path directory = extracted(temporary);
        Files.setPosixFilePermissions(directory, PosixFilePermissions.fromString("rwxrwxrwx"));
        assertNotEquals(0, probe(temporary, output).start().waitFor());
        assertTrue(Files.readString(output).contains("UnsatisfiedLinkError"), Files.readString(output));
        Files.setPosixFilePermissions(directory, PosixFilePermissions.fromString("rwx------"));
        assertEquals(0, probe(temporary, output).start().waitFor(), Files.readString(output));
    }

    /** The one directory the probes extracted the library into. */
    private static Path extracted(Path temporary) throws IOException {
        try (Stream<Path> files = Files.list(temporary)) {
            List<Path> directories = files.filter(Files::isDirectory).toList();
            assertEquals(1, directories.size(), directories.toString());
            return directories.getFirst();
        }
    }

    /** A JVM that evaluates one expression, with {@code temporary} as its temporary directory. */
    private static ProcessBuilder probe(Path temporary, Path output) {
        List<String> command = new ArrayList<>();
        command.add(ProcessHandle.current().info().command().orElseThrow());
        command.add("--enable-native-access=ALL-UNNAMED");
        command.add("-Djava.io.tmpdir=" + temporary);
        command.add("-Dstdout.encoding=" + StandardCharsets.UTF_8.name());
        command.add("-cp");
        command.add(System.getProperty("jlens.test.classes") + File.pathSeparator
                + System.getProperty("jlens.test.testClasses"));
        command.add(NativeAccessProbe.class.getName());
        return new ProcessBuilder(command).redirectErrorStream(true).redirectOutput(output.toFile());
    }
}
