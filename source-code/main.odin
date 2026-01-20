package main

import "core:fmt"
import "core:os"
import "core:strings"
import "core:path/filepath"

// Rozszerzona struktura wyniku parsowania o metadane
ParseResult_HK :: struct {
    using base: ParseResult, // Dziedziczy pola z oryginalnej struktury [cite: 1]
    project_name: string,
    version:      string,
    metadata:     map[string]string,
}

parse_hk_file :: proc(file_path: string, verbose: bool) -> ParseResult_HK {
    res: ParseResult_HK
    res.vars = make(map[string]string)
    res.local_vars = make(map[string]string)
    res.metadata = make(map[string]string)
    res.deps = make([dynamic]string)
    
    data, ok := os.read_entire_file(file_path)
    if !ok {
        append(&res.errors, fmt.tprintf("Błąd: Nie można otworzyć pliku %s", file_path)) [cite: 3]
        return res
    }
    defer delete(data)

    lines := strings.split_lines(string(data))
    current_section := ""

    for line_raw in lines {
        line := strings.trim_space(line_raw)
        
        // Ignoruj puste linie i komentarze zaczynające się od '!' 
        if line == "" || strings.has_prefix(line, "!") do continue

        // Obsługa sekcji: [nazwa]
        if strings.has_prefix(line, "[") && strings.has_suffix(line, "]") {
            current_section = line[1:len(line)-1]
            if verbose do fmt.printf("Sekcja: %s\n", current_section)
            continue
        }

        // Obsługa strzałek: -> lub -->
        if strings.has_prefix(line, "->") || strings.has_prefix(line, "-->") {
            content := ""
            if strings.has_prefix(line, "-->") {
                content = strings.trim_space(line[3:])
            } else {
                content = strings.trim_space(line[2:])
            }

            parts := strings.split(content, "=>")
            if len(parts) == 2 {
                key := strings.trim_space(parts[0])
                val := strings.trim_space(parts[1])

                switch current_section {
                case "metadata", "description":
                    res.metadata[key] = val
                    // Automatycznie ustawiamy zmienne globalne dla metadanych 
                    res.vars[strings.to_upper(key)] = val 
                
                case "specs":
                    // Traktujemy specyfikacje jako zależności systemowe [cite: 15]
                    dep_entry := fmt.tprintf("%s %s", key, val)
                    append(&res.deps, dep_entry)
                }
            }
            continue
        }
    }

    if verbose {
        fmt.printf("Sparsowane metadane: %v\n", res.metadata)
        fmt.printf("Wykryte zależności (specs): %v\n", res.deps)
    }

    return res
}

// Przykład użycia w Twoim main [cite: 66]
main :: proc() {
    // ... (reszta logiki argumentów)
    res_hk := parse_hk_file("project.hk", true)
    
    fmt.printf("Projekt: %s, Wersja: %s\n", res_hk.metadata["name"], res_hk.metadata["version"])
}
