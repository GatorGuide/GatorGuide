# Opportunity Admin Tool

The staff-facing editor is at `/(tabs)/opportunity-admin`.

- Staff access is checked by the authenticated `opportunityGateway` Function against `OPPORTUNITY_ADMIN_EMAILS` or `OPPORTUNITY_ADMIN_UIDS`.
- Create, edit, archive, restore, and delete actions modify a local in-memory draft only. They do not write to a database or publish changes.
- **Export GitHub JSON** creates the publishable `starter-opportunities.json` catalog. AI-generated college deadlines are excluded until staff review and save them as manual records. Review the export, replace `source/data/starter-opportunities.json` in the owned repository, run the applicable checks, and commit it to `main` to publish.
- The existing scholarship TSV export remains available for spreadsheet use.
- Starter-backed entries can be archived in the draft; delete remains limited to manual-only records so bundled catalog entries are not accidentally removed by the editor.

The Functions endpoint still validates manual opportunity fields and can generate a college deadline from official sources. Those calls return data to the client; the endpoint does not persist opportunity records.
