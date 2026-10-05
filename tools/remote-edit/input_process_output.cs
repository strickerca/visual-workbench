using System;
using System.Collections.Generic;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
public sealed class VwInputOutputR57 {
    public readonly ConcurrentQueue<string> Lines = new ConcurrentQueue<string>();
    private readonly string[] redact;
      private Task stdoutTask, stderrTask;
      public bool IsComplete { get { return (stdoutTask == null || stdoutTask.IsCompleted) && (stderrTask == null || stderrTask.IsCompleted); } }
      public bool OutputStarted { get { return stdoutTask != null; } }
      public bool ErrorStarted { get { return stderrTask != null; } }
    public VwInputOutputR57(string[] redactValues) {
        var variants = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (string value in redactValues ?? new string[0]) {
            if (String.IsNullOrEmpty(value)) continue;
            variants.Add(value);
            variants.Add(value.Replace("\\", "\\\\"));
            string forward = value.Replace('\\', '/');
            variants.Add(forward);
            string[] parts = forward.Split('/');
            for (int index = 0; index < parts.Length; index++) {
                // Keep the drive colon and path separators used in file URIs.
                if (!(index == 0 && Regex.IsMatch(parts[index], "^[A-Za-z]:$"))) {
                    parts[index] = Uri.EscapeDataString(parts[index]);
                }
            }
            variants.Add(String.Join("/", parts));
        }
        var ordered = new List<string>(variants);
        ordered.Sort((left, right) => right.Length.CompareTo(left.Length));
        redact = ordered.ToArray();
    }
      // Reader acquisition can fail before Task creation; the caller retains
      // explicit startup state and only settles absent readers after actual exit.
      public void StartOutput(Process process) { stdoutTask = Read(process.StandardOutput); }
      public void StartError(Process process) { stderrTask = Read(process.StandardError); }
      private Task Read(StreamReader reader) {
          return Task.Run(async () => {
              string line;
              while ((line = await reader.ReadLineAsync().ConfigureAwait(false)) != null) OnOutput(line);
          });
      }
    private void OnOutput(string line) {
        // Crash reporters sometimes print the complete process environment.
        // Keep the failure, but never retain or display that diagnostic dump.
        if (Regex.IsMatch(line, @"\benv(?:ironment)?\s*:\s*\{", RegexOptions.IgnoreCase)) {
            Lines.Enqueue("[environment diagnostic omitted]");
            return;
        }
        foreach (string value in redact) {
            line = Regex.Replace(line, Regex.Escape(value), "[redacted]", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);
        }
        Lines.Enqueue(line);
    }
}
